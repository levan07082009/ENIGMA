//! Crib attack in the style of the Turing-Welchman Bombe.
//!
//! A crib is a guessed piece of plaintext at a guessed position. For each
//! crib letter i, plaintext P_i and ciphertext C_i are linked by the
//! machine's rotor permutation at that step, M_i, through the unknown
//! plugboard S:  S(C_i) = M_i(S(P_i)).  These links form the "menu".
//!
//! For every wheel setting the Bombe assumes a plugboard partner for one
//! menu letter and follows the links. Each deduction S(a)=b also gives
//! S(b)=a (Welchman's diagonal board). A contradiction (a letter needing two
//! partners) kills the hypothesis. A setting where some hypothesis survives
//! is a "stop". It comes with part of the plugboard already deduced; the
//! rest is found by the usual hill-climb on the whole message.
//!
//! The middle rotor may step inside the crib window, so every turnover
//! position is tried too. A left-rotor step inside the window, and the
//! double-step right after a turnover in the window, are not modelled.
//! Both are rare (under ~1/26 of keys).

use crate::crack::*;
use crate::enigma::*;
use crate::stats::Lang;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct BombeOptions {
    pub rotors: Vec<usize>,
    pub reflectors: Vec<Refl>,
    /// Crib start positions (0-based) to test.
    pub positions: Vec<usize>,
    /// Also try a middle-rotor step at every point inside the crib window.
    pub turnover: bool,
    pub langs: Vec<Lang>,
    pub max_plugs: usize,
    pub threads: usize,
    pub progress: bool,
    pub part: (usize, usize),
}

pub struct BombeResult {
    pub stops: usize,
    pub solutions: Vec<Solution>,
    /// Crib position of each solution.
    pub positions: Vec<usize>,
}

/// A wheel setting that survived the menu test.
#[derive(Clone)]
struct Stop {
    pos: usize,
    order: [usize; 3],
    refl: Refl,
    /// Rotor offsets (position - ring) for the first crib letter.
    off: [u8; 3],
    /// Middle rotor steps before crib letter `turn` (crib length = none).
    turn: usize,
    /// Deduced plugboard partner per letter, 255 = unknown.
    stecker: [u8; 26],
}

struct Menu {
    /// adj[letter] = (other letter, crib index) for every link.
    adj: [Vec<(u8, u8)>; 26],
    test_letter: usize,
    /// One letter from each other connected part of the menu.
    others: Vec<usize>,
}

impl Menu {
    fn new(crib: &[u8], ct: &[u8]) -> Menu {
        let mut adj: [Vec<(u8, u8)>; 26] = std::array::from_fn(|_| vec![]);
        for (i, (&p, &c)) in crib.iter().zip(ct).enumerate() {
            adj[p as usize].push((c, i as u8));
            adj[c as usize].push((p, i as u8));
        }
        let test_letter = (0..26).max_by_key(|&l| adj[l].len()).unwrap();
        // Connected parts of the menu, largest-first test letter excluded.
        let mut comp = [usize::MAX; 26];
        let mut others = vec![];
        let mut order: Vec<usize> = (0..26).filter(|&l| !adj[l].is_empty()).collect();
        order.sort_by_key(|&l| (l != test_letter, std::cmp::Reverse(adj[l].len())));
        for (ci, &root) in order.iter().enumerate() {
            if comp[root] != usize::MAX {
                continue;
            }
            let mut stack = vec![root];
            comp[root] = ci;
            while let Some(a) = stack.pop() {
                for &(b, _) in &adj[a] {
                    if comp[b as usize] == usize::MAX {
                        comp[b as usize] = ci;
                        stack.push(b as usize);
                    }
                }
            }
            if root != test_letter {
                others.push(root);
            }
        }
        Menu { adj, test_letter, others }
    }
}

/// Follow the menu from S(start) = h, adding to the deductions already in
/// `st`. `perm(i, x)` is the rotor permutation at crib index i. Returns
/// false on contradiction.
#[inline(always)]
fn propagate(menu: &Menu, start: usize, h: u8, st: &mut [u8; 26], perm: impl Fn(usize, u8) -> u8) -> bool {
    let mut stack = [0u8; 52];
    let mut sp = 0;
    macro_rules! set {
        ($a:expr, $b:expr) => {{
            let (a, b) = ($a as usize, $b as usize);
            if st[a] == 255 {
                if st[b] != 255 {
                    return false; // b already has a different partner
                }
                st[a] = b as u8;
                st[b] = a as u8;
                stack[sp] = a as u8;
                sp += 1;
                if a != b {
                    stack[sp] = b as u8;
                    sp += 1;
                }
            } else if st[a] as usize != b {
                return false;
            }
        }};
    }
    set!(start, h);
    while sp > 0 {
        sp -= 1;
        let a = stack[sp] as usize;
        let s = st[a];
        for &(other, i) in &menu.adj[a] {
            set!(other, perm(i as usize, s));
        }
    }
    true
}

fn cables_deduced(st: &[u8; 26]) -> usize {
    (0..26).filter(|&a| st[a] != 255 && st[a] as usize > a).count()
}

pub fn bombe(ct: &[u8], crib: &[u8], opts: &BombeOptions) -> BombeResult {
    let len = crib.len();
    let orders = rotor_orders(&opts.rotors);
    let mut items = vec![];
    for &p in &opts.positions {
        for o in &orders {
            for &r in &opts.reflectors {
                items.push((p, *o, r));
            }
        }
    }
    let (k, parts) = opts.part;
    let items: Vec<_> = items.into_iter().enumerate().filter(|(i, _)| i % parts == k - 1).map(|(_, x)| x).collect();
    let turns: Vec<usize> = if opts.turnover { (1..=len).collect() } else { vec![len] };
    let menus: Vec<Menu> = (0..ct.len())
        .map(|p| if p + len <= ct.len() { Menu::new(crib, &ct[p..p + len]) } else { Menu::new(&[], &[]) })
        .collect();
    let stop_count = AtomicUsize::new(0);

    // Each stop is completed into full keys right away and only the best
    // results are kept, so no stop is ever dropped.
    let found = parallel(
        items.len(),
        opts.threads,
        "bombe",
        opts.progress,
        Vec::<(Solution, usize)>::new,
        |results, idx| {
            let (p, order, refl_i) = items[idx];
            let menu = &menus[p];
            let rot: [Rotor; 3] = std::array::from_fn(|i| Rotor::new(order[i]));
            let refl = refl_i.wiring();
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
            for off_r in 0..26 {
                let rf: Vec<&[u8; 26]> = (0..len).map(|i| &rot[2].fwd[(off_r + i) % 26]).collect();
                let rb: Vec<&[u8; 26]> = (0..len).map(|i| &rot[2].bwd[(off_r + i) % 26]).collect();
                for off_l in 0..26 {
                    for off_m in 0..26 {
                        let row_a = &inner[(off_l * 26 + off_m) * 26..][..26];
                        let row_b = &inner[(off_l * 26 + (off_m + 1) % 26) * 26..][..26];
                        for &turn in &turns {
                            let perm = |i: usize, x: u8| {
                                let row = if i >= turn { row_b } else { row_a };
                                rb[i][row[rf[i][x as usize] as usize] as usize]
                            };
                            for h in 0..26u8 {
                                let mut st = [255u8; 26];
                                if !propagate(menu, menu.test_letter, h, &mut st, perm)
                                    || cables_deduced(&st) > opts.max_plugs
                                {
                                    continue;
                                }
                                // Every other part of the menu must also admit
                                // a plugboard partner consistent with these
                                // deductions (checked independently, so the
                                // true setting is never rejected).
                                let others_ok = menu.others.iter().all(|&c| {
                                    st[c] != 255
                                        || (0..26u8).any(|h2| {
                                            let mut t = st;
                                            propagate(menu, c, h2, &mut t, perm) && cables_deduced(&t) <= opts.max_plugs
                                        })
                                });
                                if !others_ok {
                                    continue;
                                }
                                stop_count.fetch_add(1, Ordering::Relaxed);
                                let stop = Stop {
                                    pos: p,
                                    order,
                                    refl: refl_i,
                                    off: [off_l as u8, off_m as u8, off_r as u8],
                                    turn,
                                    stecker: st,
                                };
                                complete(ct, crib, &stop, opts, results);
                                if results.len() > 200 {
                                    results.sort_by(|a, b| b.0.fitness.total_cmp(&a.0.fitness));
                                    results.truncate(50);
                                }
                            }
                        }
                    }
                }
            }
        },
    );
    let mut all: Vec<(Solution, usize)> = found.into_iter().flatten().collect();
    all.sort_by(|a, b| b.0.fitness.total_cmp(&a.0.fitness));
    let mut seen = std::collections::HashSet::new();
    all.retain(|(s, _)| seen.insert(s.plaintext.clone()));
    all.truncate(20);
    let (solutions, positions) = all.into_iter().unzip();
    BombeResult { stops: stop_count.load(Ordering::Relaxed), solutions, positions }
}

/// Turn a stop into full keys: every consistent ring/start setting, with
/// the rest of the plugboard hill-climbed on the whole message.
fn complete(ct: &[u8], crib: &[u8], stop: &Stop, opts: &BombeOptions, out: &mut Vec<(Solution, usize)>) {
    let len = crib.len();
    let rot: [Rotor; 3] = std::array::from_fn(|i| Rotor::new(stop.order[i]));
    let mut locked = [false; 26];
    let mut plug = identity_plug();
    for a in 0..26 {
        if stop.stecker[a] != 255 {
            locked[a] = true;
            plug[a] = stop.stecker[a];
        }
    }
    let mut buf = vec![0u8; ct.len()];
    for s in full_settings(stop, len, ct.len(), &rot) {
        let mut s = Settings { plug, ..s };
        let core = core_perms(&s, ct.len());
        // The Bombe only locks cables in the test letter's part of the menu;
        // the climb finds the rest, strongly rewarded for reproducing the crib.
        let crib_bonus =
            |t: &[u8]| -> i32 { t[stop.pos..stop.pos + len].iter().zip(crib).filter(|(a, b)| a == b).count() as i32 * 10_000 };
        for lang in &opts.langs {
            let mut q = plug;
            climb_locked(ct, &core, &mut q, &locked, opts.max_plugs, &mut buf, CABLE_PENALTY, |t| {
                lang.trigram_score(t) + crib_bonus(t)
            });
            apply(ct, &core, &q, &mut buf);
            if buf[stop.pos..stop.pos + len] != *crib {
                continue; // an un-modelled rotor step broke the crib
            }
            s.plug = q;
            let fit = fitness(lang, &buf);
            out.push((
                Solution { settings: s.clone(), lang: lang.name, fitness: fit, rank: fit, plaintext: buf.clone() },
                stop.pos,
            ));
        }
    }
}

/// All message-start settings that reproduce the stop's rotor offsets over
/// the crib window. The right and middle rings decide when the middle and
/// left rotors step before and after the crib, so all 676 are tried.
/// Settings that move the rotors identically over the whole message are
/// kept only once.
fn full_settings(stop: &Stop, len: usize, n: usize, rot: &[Rotor; 3]) -> Vec<Settings> {
    let p = stop.pos;
    let target = |i: usize| -> [u8; 3] {
        [
            stop.off[0],
            (stop.off[1] + (i >= stop.turn) as u8) % 26,
            ((stop.off[2] as usize + i) % 26) as u8,
        ]
    };
    // Offset of the right rotor for letter k is start_offset + k + 1.
    let start_r = ((stop.off[2] as usize + 26 - (p + 1) % 26) % 26) as u8;
    let steps = n.max(p + len);
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    for ring_r in 0..26u8 {
        for ring_m in 0..26u8 {
            let rings = [0, ring_m, ring_r];
            let offsets = |start: [u8; 3]| -> Vec<[u8; 3]> {
                let mut pos: [u8; 3] = std::array::from_fn(|k| (start[k] + rings[k]) % 26);
                let mut v = Vec::with_capacity(steps);
                for _ in 0..steps {
                    step(&mut pos, &rot[1].notch, &rot[2].notch);
                    v.push(std::array::from_fn(|k| (pos[k] + 26 - rings[k]) % 26));
                }
                v
            };
            // Count the left/middle steps up to the crib, then shift the
            // start so the offsets line up; re-check because the double step
            // depends on the middle rotor's actual position.
            let probe = offsets([0, 0, start_r]);
            let start = [
                (stop.off[0] + 26 - probe[p][0]) % 26,
                (stop.off[1] + 26 - probe[p][1]) % 26,
                start_r,
            ];
            let got = offsets(start);
            if (0..len).all(|i| got[p + i] == target(i)) && seen.insert(got) {
                out.push(Settings {
                    rotors: stop.order,
                    reflector: stop.refl,
                    rings,
                    pos: std::array::from_fn(|k| (start[k] + rings[k]) % 26),
                    plug: identity_plug(),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(rotors: Vec<usize>, reflectors: Vec<Refl>, positions: Vec<usize>) -> BombeOptions {
        BombeOptions {
            rotors,
            reflectors,
            positions,
            turnover: true,
            langs: vec![Lang::german()],
            max_plugs: 10,
            threads: 4,
            progress: false,
            part: (1, 1),
        }
    }

    #[test]
    fn breaks_m3_with_full_plugboard() {
        let text = to_letters(crate::DEMO_DE);
        let pt = &text[..160];
        let key = Settings {
            rotors: [2, 0, 4],
            reflector: Refl::Std(1),
            rings: [5, 17, 9],
            pos: [11, 3, 20],
            plug: parse_plugs("AQ BW CE DR FT GZ HU IJ KO LP").unwrap(),
        };
        let ct = run(&key, pt);
        let crib = to_letters("WETTERBERICHT");
        let at = to_string(pt).find("WETTERBERICHT").unwrap();
        let r = bombe(&ct, &crib, &opts(vec![2, 0, 4], vec![Refl::Std(1)], vec![at]));
        assert!(r.stops >= 1);
        assert_eq!(r.solutions[0].plaintext, pt, "stops {}", r.stops);
    }

    #[test]
    fn breaks_u264_m4() {
        // U-264 (1942), Beta II IV I, thin B: crib "BEIANGRIFFUNTERWASSER".
        let ct = to_letters(
            "NCZWVUSXPNYMINHZXMQXSFWXWLKJAHSHNMCOCCAKUQPMKCSMHKSEINJUSBLKIOSXCKUBHMLLXCSJUSRRDVKOHULXWCCBGVLIYXEOAHXRHKKFVDREWEZLXOBAFGYUJQUKGRTVUKAMEURBVEKSUHHVOYHABCJWMAKLFKLMYFVNRIZRVVRTKOFDANJMOLBGFFLEOPRGTFLVRHOWOPBEKVWMUQFMPWPARMFHAGKXIIBG",
        );
        let crib = to_letters("BEIANGRIFFUNTERWASSER");
        let refls: Vec<Refl> = (0..26).map(|off| Refl::M4 { greek: 0, thin: 0, off }).collect();
        let r = bombe(&ct, &crib, &opts(vec![1, 3, 0], refls, vec![52]));
        let pt = to_string(&r.solutions[0].plaintext);
        assert!(pt.starts_with("VONVONJLOOKSJHFFTTTEINSEINSDREIZWO"), "{pt}");
    }
}

