//! Ciphertext-only attack on Enigma I / M3.
//!
//! Phase 1 tries every rotor order, reflector and start position with an
//! empty plugboard. For short messages (`full_rings`) it also tries every
//! distinct rotor-stepping pattern the ring settings can produce. Each trial
//! is scored by trigram statistics (one list per language) and, with default
//! rings, by index of coincidence, which survives an unknown plugboard
//! better. The best few thousand of each list are kept.
//!
//! Phase 2 hill-climbs the plugboard for every kept candidate (IoC, then
//! bigrams, then trigrams), refines the ring settings, and ranks the results
//! by how language-like they read.

use crate::enigma::*;
use crate::stats::*;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

pub struct Options {
    pub rotors: Vec<usize>,
    pub reflectors: Vec<usize>,
    pub langs: Vec<Lang>,
    /// Candidates kept per metric after phase 1.
    pub keep: usize,
    /// Maximum plugboard pairs in phase 2 (0 skips plugboard search).
    pub max_plugs: usize,
    /// Try every ring setting in phase 1 (needed for short messages).
    pub full_rings: bool,
    pub threads: usize,
    pub progress: bool,
}

pub struct Solution {
    pub settings: Settings,
    pub lang: &'static str,
    /// 0 = reads like random letters, 1 = reads like typical text.
    pub fitness: f64,
    /// Fitness after charging each plugboard cable `CABLE_PENALTY`; used for
    /// ranking so extra cables must earn their place.
    pub rank: f64,
    pub plaintext: Vec<u8>,
}

/// A phase-1 hit, stored compactly: rotor order, reflector, rotor offsets
/// at the start (position - ring) and the right/middle ring settings.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Cand {
    order: [u8; 3],
    refl: u8,
    off: [u8; 3],
    ring_m: u8,
    ring_r: u8,
}

impl Cand {
    fn settings(&self) -> Settings {
        Settings {
            rotors: self.order.map(|r| r as usize),
            reflector: self.refl as usize,
            rings: [0, self.ring_m, self.ring_r],
            pos: [
                self.off[0],
                (self.off[1] + self.ring_m) % 26,
                (self.off[2] + self.ring_r) % 26,
            ],
            plug: identity_plug(),
        }
    }
}

struct TopK {
    k: usize,
    heap: BinaryHeap<Reverse<(i32, Cand)>>,
}

impl TopK {
    fn new(k: usize) -> TopK {
        TopK { k, heap: BinaryHeap::with_capacity(k + 1) }
    }

    #[inline(always)]
    fn push(&mut self, score: i32, c: Cand) {
        if self.heap.len() < self.k {
            self.heap.push(Reverse((score, c)));
        } else if score > self.heap.peek().unwrap().0.0 {
            self.heap.pop();
            self.heap.push(Reverse((score, c)));
        }
    }
}

fn fitness(lang: &Lang, pt: &[u8]) -> f64 {
    (lang.per_letter(pt) - lang.random) / (lang.typical - lang.random)
}

/// Run `work(i)` for i in 0..n on all threads, collecting per-thread state.
fn parallel<S: Send>(
    n: usize,
    threads: usize,
    label: &str,
    progress: bool,
    init: impl Fn() -> S + Sync,
    work: impl Fn(&mut S, usize) + Sync,
) -> Vec<S> {
    let next = AtomicUsize::new(0);
    let start = Instant::now();
    let out = Mutex::new(vec![]);
    let last_print = AtomicU64::new(0);
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| {
                let mut state = init();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= n {
                        break;
                    }
                    work(&mut state, i);
                    let ms = start.elapsed().as_millis() as u64;
                    let last = last_print.load(Ordering::Relaxed);
                    if progress
                        && ms >= last + 1000
                        && last_print
                            .compare_exchange(last, ms, Ordering::Relaxed, Ordering::Relaxed)
                            .is_ok()
                    {
                        eprint!(
                            "\r  {label}: {:5.1}%  ({:.0}s)   ",
                            100.0 * (i + 1) as f64 / n as f64,
                            start.elapsed().as_secs_f64()
                        );
                    }
                }
                out.lock().unwrap().push(state);
            });
        }
    });
    if progress {
        eprintln!("\r  {label}: done in {:.1}s          ", start.elapsed().as_secs_f64());
    }
    out.into_inner().unwrap()
}

fn rotor_orders(rotors: &[usize]) -> Vec<[usize; 3]> {
    let mut v = vec![];
    for &a in rotors {
        for &b in rotors {
            for &c in rotors {
                if a != b && b != c && a != c {
                    v.push([a, b, c]);
                }
            }
        }
    }
    v
}


pub fn crack(ct: &[u8], opts: &Options) -> Vec<Solution> {
    let n = ct.len();
    let orders = rotor_orders(&opts.rotors);
    let mut items = vec![];
    for o in &orders {
        for &r in &opts.reflectors {
            for off_m in 0..26u8 {
                items.push((*o, r, off_m));
            }
        }
    }
    let metrics = opts.langs.len() + 1; // one per language + IoC
    let ioc_metric = metrics - 1;
    // Ring settings to try in phase 1. Only the middle and right rings matter
    // (the left one just shifts the left rotor), and only through the
    // stepping pattern they cause. For each right ring the middle ring A
    // comes first, so it is the representative for its pattern.
    let ring_pairs: Vec<(u8, u8)> = if opts.full_rings {
        (0..26u8).flat_map(|r| (0..26u8).map(move |m| (r, m))).collect()
    } else {
        (0..26u8).map(|r| (r, 0)).collect()
    };

    // ---------------- Phase 1: rotor search, no plugboard ----------------
    let states = parallel(
        items.len(),
        opts.threads,
        "phase 1/2 rotor search",
        opts.progress,
        || (0..metrics).map(|_| TopK::new(opts.keep)).collect::<Vec<_>>(),
        |tops, idx| {
            let (order, refl_i, off_m) = items[idx];
            let rot: [Rotor; 3] = std::array::from_fn(|i| Rotor::new(order[i]));
            let refl = reflector(refl_i);
            // L, M and reflector folded into one table per (left, middle) offset.
            let mut inner = vec![0u8; 26 * 26 * 26];
            for l in 0..26 {
                for m in 0..26 {
                    for x in 0..26 {
                        let mut y = rot[1].fwd[m][x] as usize;
                        y = rot[0].fwd[l][y] as usize;
                        y = refl[y] as usize;
                        y = rot[0].bwd[l][y] as usize;
                        y = rot[1].bwd[m][y] as usize;
                        inner[(l * 26 + m) * 26 + x] = y as u8;
                    }
                }
            }
            let mut buf = vec![0u8; n];
            let mut left_steps = vec![0u8; n];
            let mut mid_off = vec![0u8; n];
            for off_r in 0..26u8 {
                // The left rotor's offset never affects stepping, so collect
                // the distinct (left-steps, middle-offset) sequences that the
                // ring settings produce and decrypt each only once.
                let mut seen: HashSet<Vec<u16>> = HashSet::new();
                let mut seqs = vec![];
                for &(ring_r, ring_m) in &ring_pairs {
                    let mut pos = [0u8, (off_m + ring_m) % 26, (off_r + ring_r) % 26];
                    let mut seq = Vec::with_capacity(n);
                    for _ in 0..n {
                        step(&mut pos, &rot[1].notch, &rot[2].notch);
                        seq.push(pos[0] as u16 * 26 + ((pos[1] + 26 - ring_m) % 26) as u16);
                    }
                    if seen.insert(seq.clone()) {
                        seqs.push((seq, ring_r, ring_m));
                    }
                }
                let right: Vec<usize> = (0..n).map(|i| (off_r as usize + i + 1) % 26).collect();
                for (seq, ring_r, ring_m) in seqs {
                    for i in 0..n {
                        left_steps[i] = (seq[i] / 26) as u8;
                        mid_off[i] = (seq[i] % 26) as u8;
                    }
                    for off_l in 0..26u8 {
                        for i in 0..n {
                            let r = right[i];
                            let mut l = off_l + left_steps[i];
                            if l >= 26 {
                                l -= 26;
                            }
                            let x = rot[2].fwd[r][ct[i] as usize] as usize;
                            let y = inner[(l as usize * 26 + mid_off[i] as usize) * 26 + x];
                            buf[i] = rot[2].bwd[r][y as usize];
                        }
                        let c = Cand {
                            order: order.map(|r| r as u8),
                            refl: refl_i as u8,
                            off: [off_l, off_m, off_r],
                            ring_m,
                            ring_r,
                        };
                        for (li, lang) in opts.langs.iter().enumerate() {
                            tops[li].push(lang.trigram_score(&buf), c);
                        }
                        // IoC is the plugboard-tolerant signal. The middle ring
                        // only moves the rare left-rotor step, so leave it at A.
                        if ring_m == 0 {
                            tops[ioc_metric].push(ioc_score(&buf), c);
                        }
                    }
                }
            }
        },
    );

    // Candidate -> whether it came from the IoC list.
    let mut cands: HashMap<Cand, bool> = HashMap::new();
    for mut tops in states {
        for (mi, t) in tops.iter_mut().enumerate() {
            for Reverse((_, c)) in t.heap.drain() {
                *cands.entry(c).or_insert(false) |= mi == ioc_metric;
            }
        }
    }
    let cands: Vec<(Cand, bool)> = cands.into_iter().collect();

    // ---------------- Phase 2: plugboard hill-climb + ring refinement ----------------
    let results = parallel(
        cands.len(),
        opts.threads,
        "phase 2/2 plugboard search",
        opts.progress,
        Vec::<Solution>::new,
        |sols, idx| {
            let (cand, from_ioc) = cands[idx];
            for lang in &opts.langs {
                sols.push(solve_candidate(ct, cand, from_ioc, lang, opts.max_plugs));
            }
            // Keep per-thread memory small.
            if sols.len() > 400 {
                sols.sort_by(|a, b| b.rank.total_cmp(&a.rank));
                sols.truncate(100);
            }
        },
    );

    let mut all: Vec<Solution> = results.into_iter().flatten().collect();
    all.sort_by(|a, b| b.rank.total_cmp(&a.rank));
    let mut seen = HashSet::new();
    all.retain(|s| seen.insert(s.plaintext.clone()));
    all.truncate(20);
    all
}

/// Phase 2 for one rotor candidate: plugboard hill-climb, ring refinement,
/// and a final climb if the rings moved.
fn solve_candidate(ct: &[u8], cand: Cand, from_ioc: bool, lang: &Lang, max_plugs: usize) -> Solution {
    let n = ct.len();
    let rot: [Rotor; 3] = std::array::from_fn(|i| Rotor::new(cand.order[i] as usize));
    let refl = reflector(cand.refl as usize);
    let mut buf = vec![0u8; n];
    let mut s = cand.settings();
    let core = core_perms(&s, n);
    if max_plugs > 0 {
        if from_ioc {
            climb(ct, &core, &mut s.plug, max_plugs, &mut buf, 0, ioc_score);
        }
        climb(ct, &core, &mut s.plug, max_plugs, &mut buf, CABLE_PENALTY, |p| lang.bigram_score(p));
        climb(ct, &core, &mut s.plug, max_plugs, &mut buf, CABLE_PENALTY, |p| lang.trigram_score(p));
    }
    if refine_rings(ct, &rot, &refl, &mut s, lang, &mut buf) && max_plugs > 0 {
        let core = core_perms(&s, n);
        climb(ct, &core, &mut s.plug, max_plugs, &mut buf, CABLE_PENALTY, |p| lang.trigram_score(p));
    }
    run_with(&rot, &refl, &s, ct, &mut buf);
    let fit = fitness(lang, &buf);
    let penalty = (cables(&s.plug) as i32 * CABLE_PENALTY) as f64 / 1000.0 / (n.max(3) - 2) as f64;
    let rank = fit - penalty / (lang.typical - lang.random);
    Solution { fitness: fit, rank, lang: lang.name, settings: s, plaintext: buf }
}

/// Encrypt/decrypt with pre-built rotors (avoids rebuilding tables).
fn run_with(rot: &[Rotor; 3], refl: &[u8; 26], s: &Settings, ct: &[u8], out: &mut [u8]) {
    let mut pos = s.pos;
    for (i, &c) in ct.iter().enumerate() {
        step(&mut pos, &rot[1].notch, &rot[2].notch);
        let off: [usize; 3] = std::array::from_fn(|k| ((pos[k] + 26 - s.rings[k]) % 26) as usize);
        let mut x = s.plug[c as usize] as usize;
        x = rot[2].fwd[off[2]][x] as usize;
        x = rot[1].fwd[off[1]][x] as usize;
        x = rot[0].fwd[off[0]][x] as usize;
        x = refl[x] as usize;
        x = rot[0].bwd[off[0]][x] as usize;
        x = rot[1].bwd[off[1]][x] as usize;
        x = rot[2].bwd[off[2]][x] as usize;
        out[i] = s.plug[x];
    }
}

/// Try all middle/right ring settings (keeping the rotor offsets, i.e. the
/// wiring alignment, fixed) and keep the one whose stepping reads best.
/// Returns true if the settings changed.
fn refine_rings(
    ct: &[u8],
    rot: &[Rotor; 3],
    refl: &[u8; 26],
    s: &mut Settings,
    lang: &Lang,
    buf: &mut [u8],
) -> bool {
    let off: [u8; 3] = std::array::from_fn(|k| (s.pos[k] + 26 - s.rings[k]) % 26);
    run_with(rot, refl, s, ct, buf);
    let mut best = (lang.trigram_score(buf), s.clone());
    let mut t = s.clone();
    for ring_r in 0..26u8 {
        for ring_m in 0..26u8 {
            t.rings = [0, ring_m, ring_r];
            t.pos = [off[0], (off[1] + ring_m) % 26, (off[2] + ring_r) % 26];
            run_with(rot, refl, &t, ct, buf);
            let sc = lang.trigram_score(buf);
            if sc > best.0 {
                best = (sc, t.clone());
            }
        }
    }
    let changed = best.1 != *s;
    *s = best.1;
    changed
}

/// Score cost of one plugboard cable (log10 x 1000 units, i.e. a cable must
/// make the text about 10^4 times more likely). Without it, a short message lets
/// the climb add bogus cables that fit the statistics a little better.
const CABLE_PENALTY: i32 = 4000;

fn cables(plug: &[u8; 26]) -> usize {
    (0..26).filter(|&k| (plug[k] as usize) > k).count()
}

/// Greedy plugboard hill-climb. For every letter pair it tries connecting,
/// disconnecting and re-routing existing cables, keeping any improvement,
/// until a full pass finds nothing better.
pub fn climb(
    ct: &[u8],
    core: &[[u8; 26]],
    plug: &mut [u8; 26],
    max_pairs: usize,
    buf: &mut [u8],
    penalty: i32,
    score: impl Fn(&[u8]) -> i32,
) -> i32 {
    let eval = |p: &[u8; 26], buf: &mut [u8]| {
        apply(ct, core, p, buf);
        score(buf) - penalty * cables(p) as i32
    };
    let mut best = eval(plug, buf);
    loop {
        let mut improved = false;
        for i in 0..26 {
            for j in i + 1..26 {
                for q in moves(plug, i, j, max_pairs).into_iter().flatten() {
                    let s = eval(&q, buf);
                    if s > best {
                        best = s;
                        *plug = q;
                        improved = true;
                        break;
                    }
                }
            }
        }
        if !improved {
            return best;
        }
    }
}

fn moves(p: &[u8; 26], i: usize, j: usize, max_pairs: usize) -> [Option<[u8; 26]>; 3] {
    let pairs = cables(p);
    let (pi, pj) = (p[i] as usize, p[j] as usize);
    let mut out = [None; 3];
    let set = |q: &mut [u8; 26], a: usize, b: usize| {
        q[a] = b as u8;
        q[b] = a as u8;
    };
    if pi == j {
        let mut q = *p;
        set(&mut q, i, i);
        set(&mut q, j, j);
        out[0] = Some(q);
    } else if pi == i && pj == j {
        if pairs < max_pairs {
            let mut q = *p;
            set(&mut q, i, j);
            out[0] = Some(q);
        }
    } else if pi == i || pj == j {
        // Exactly one of i, j is plugged (to x); the other is free.
        let (a, b, x) = if pi != i { (i, j, pi) } else { (j, i, pj) };
        let mut q = *p;
        q[x] = x as u8;
        set(&mut q, a, b); // a-b, x free
        out[0] = Some(q);
        let mut q = *p;
        q[a] = a as u8;
        set(&mut q, x, b); // x-b, a free
        out[1] = Some(q);
    } else {
        let (x, y) = (pi, pj);
        let mut q = *p;
        set(&mut q, i, j);
        set(&mut q, x, y);
        out[0] = Some(q);
        let mut q = *p;
        set(&mut q, i, y);
        set(&mut q, j, x);
        out[1] = Some(q);
        let mut q = *p;
        set(&mut q, i, j);
        q[x] = x as u8;
        q[y] = y as u8;
        out[2] = Some(q);
    }
    out
}

/// Positions where a known plaintext word could sit: Enigma never encrypts
/// a letter to itself, so any position with a coincidence is impossible.
pub fn crib_positions(ct: &[u8], crib: &[u8]) -> Vec<usize> {
    if crib.len() > ct.len() {
        return vec![];
    }
    (0..=ct.len() - crib.len())
        .filter(|&p| crib.iter().enumerate().all(|(k, &c)| ct[p + k] != c))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Diagnostic: start phase 2 from the true rotor offsets (rings reset to
    /// the phase-1 default) and check the plugboard is recovered.
    #[test]
    fn phase2_from_true_offsets() {
        let text = to_letters(crate::DEMO_DE);
        let key = Settings {
            rotors: [4, 1, 3],
            reflector: 1,
            rings: [20, 3, 19],
            pos: [2, 20, 2],
            plug: parse_plugs("AI BM ES FL GK HP JQ NR VY WZ").unwrap(),
        };
        let pt = &text[..150];
        let ct = run(&key, pt);
        let off: [u8; 3] = std::array::from_fn(|k| (key.pos[k] + 26 - key.rings[k]) % 26);
        let cand = Cand { order: key.rotors.map(|r| r as u8), refl: 1, off, ring_m: key.rings[1], ring_r: key.rings[2] };
        let lang = Lang::german();
        let sol = solve_candidate(&ct, cand, true, &lang, 10);
        eprintln!("fitness {:.2} plug {} rings {:?}", sol.fitness, plug_pairs(&sol.settings.plug), sol.settings.rings);
        eprintln!("{}", to_string(&sol.plaintext));
        assert_eq!(sol.plaintext, pt);
    }
}

