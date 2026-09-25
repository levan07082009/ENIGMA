//! Enigma I / M3 / M4 machine model: rotors I-VIII, reflectors A/B/C, the
//! M4's Greek wheels (Beta, Gamma) with thin reflectors, ring settings,
//! plugboard and the double-stepping anomaly.
//!
//! The M4's Greek wheel never steps, so Greek wheel + thin reflector act as
//! one fixed reflector. An M4 is therefore modelled as a three-rotor machine
//! whose reflector is `Refl::M4 { greek, thin, off }`, where `off` is the
//! Greek wheel's position minus its ring setting.

pub const ROTORS: [(&str, &str, &str); 8] = [
    ("I", "EKMFLGDQVZNTOWYHXUSPAIBRCJ", "Q"),
    ("II", "AJDKSIRUXBLHWTMCQGZNPYFVOE", "E"),
    ("III", "BDFHJLCPRTXVZNYEIWGAKMUSQO", "V"),
    ("IV", "ESOVPZJAYQUIRHXLNFTGKDCMWB", "J"),
    ("V", "VZBRGITYUPSDNHLXAWMJQOFECK", "Z"),
    ("VI", "JPGVOUMFYQBENHZRDKASXLICTW", "ZM"),
    ("VII", "NZJHGRCXMYSWBOUFAIVLPEKQDT", "ZM"),
    ("VIII", "FKQHTLXOCBJSPDZRAMEWNIUYGV", "ZM"),
];

pub const REFLECTORS: [(&str, &str); 3] = [
    ("A", "EJMZALYXVBWFCRQUONTSPIKHGD"),
    ("B", "YRUHQSLDPXNGOKMIEBFZCWVJAT"),
    ("C", "FVPJIAOYEDRZXWGCTKUQSBNMHL"),
];

/// M4 Greek wheels (fourth rotor, never steps).
pub const GREEK: [(&str, &str); 2] = [
    ("Beta", "LEYJVCNIXWPBQMDRTAKZGFUHOS"),
    ("Gamma", "FSOKANUERHMBTIYCWLQPZXVGJD"),
];

/// M4 thin reflectors (UKW-b, UKW-c).
pub const THIN: [(&str, &str); 2] = [
    ("B", "ENKQAUYWJICOPBLMDXZVFTHRGS"),
    ("C", "RDOBJNTKVEHMLFCWZAXGYIPSUQ"),
];

/// A rotor with its wiring pre-shifted for every offset (position - ring),
/// so a pass through it is a single table lookup.
pub struct Rotor {
    pub fwd: [[u8; 26]; 26],
    pub bwd: [[u8; 26]; 26],
    pub notch: [bool; 26],
}

impl Rotor {
    pub fn new(index: usize) -> Rotor {
        let (_, wiring, notches) = ROTORS[index];
        Rotor::from_wiring(wiring, notches)
    }

    pub fn from_wiring(wiring: &str, notches: &str) -> Rotor {
        let w: Vec<u8> = wiring.bytes().map(|b| b - b'A').collect();
        let mut inv = [0u8; 26];
        for (i, &o) in w.iter().enumerate() {
            inv[o as usize] = i as u8;
        }
        let mut fwd = [[0u8; 26]; 26];
        let mut bwd = [[0u8; 26]; 26];
        for o in 0..26 {
            for c in 0..26 {
                fwd[o][c] = ((w[(c + o) % 26] as usize + 26 - o) % 26) as u8;
                bwd[o][c] = ((inv[(c + o) % 26] as usize + 26 - o) % 26) as u8;
            }
        }
        let mut notch = [false; 26];
        for b in notches.bytes() {
            notch[(b - b'A') as usize] = true;
        }
        Rotor { fwd, bwd, notch }
    }
}

fn wiring_table(w: &str) -> [u8; 26] {
    let mut r = [0u8; 26];
    for (i, b) in w.bytes().enumerate() {
        r[i] = b - b'A';
    }
    r
}

/// The reflecting end of the machine: a plain M3 reflector, or an M4 Greek
/// wheel (at offset `off` = position - ring) in front of a thin reflector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Refl {
    Std(u8),
    M4 { greek: u8, thin: u8, off: u8 },
}

impl Refl {
    pub fn wiring(&self) -> [u8; 26] {
        match *self {
            Refl::Std(i) => wiring_table(REFLECTORS[i as usize].1),
            Refl::M4 { greek, thin, off } => {
                let g = Rotor::from_wiring(GREEK[greek as usize].1, "");
                let t = wiring_table(THIN[thin as usize].1);
                let o = off as usize;
                std::array::from_fn(|c| g.bwd[o][t[g.fwd[o][c] as usize] as usize])
            }
        }
    }

    pub fn is_m4(&self) -> bool {
        matches!(self, Refl::M4 { .. })
    }
}

/// A complete daily key + message start position.
/// Arrays are ordered left, middle, right.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub rotors: [usize; 3],
    pub reflector: Refl,
    pub rings: [u8; 3],
    pub pos: [u8; 3],
    pub plug: [u8; 26],
}

pub fn identity_plug() -> [u8; 26] {
    std::array::from_fn(|i| i as u8)
}

/// Advance the rotors before a key press, including the middle rotor's
/// double step.
#[inline(always)]
pub fn step(pos: &mut [u8; 3], notch_m: &[bool; 26], notch_r: &[bool; 26]) {
    if notch_m[pos[1] as usize] {
        pos[0] = (pos[0] + 1) % 26;
        pos[1] = (pos[1] + 1) % 26;
    } else if notch_r[pos[2] as usize] {
        pos[1] = (pos[1] + 1) % 26;
    }
    pos[2] = (pos[2] + 1) % 26;
}

/// The rotor+reflector permutation (plugboard excluded) for each of the
/// first `n` key presses. Enciphering letter i is then
/// `plug[core[i][plug[c]]]`, which lets the plugboard be varied cheaply.
pub fn core_perms(s: &Settings, n: usize) -> Vec<[u8; 26]> {
    let rot: [Rotor; 3] = std::array::from_fn(|i| Rotor::new(s.rotors[i]));
    let refl = s.reflector.wiring();
    let mut pos = s.pos;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        step(&mut pos, &rot[1].notch, &rot[2].notch);
        let off: [usize; 3] =
            std::array::from_fn(|i| ((pos[i] + 26 - s.rings[i]) % 26) as usize);
        let mut perm = [0u8; 26];
        for (c, slot) in perm.iter_mut().enumerate() {
            let mut x = c;
            x = rot[2].fwd[off[2]][x] as usize;
            x = rot[1].fwd[off[1]][x] as usize;
            x = rot[0].fwd[off[0]][x] as usize;
            x = refl[x] as usize;
            x = rot[0].bwd[off[0]][x] as usize;
            x = rot[1].bwd[off[1]][x] as usize;
            x = rot[2].bwd[off[2]][x] as usize;
            *slot = x as u8;
        }
        out.push(perm);
    }
    out
}

#[inline(always)]
pub fn apply(ct: &[u8], core: &[[u8; 26]], plug: &[u8; 26], out: &mut [u8]) {
    for i in 0..ct.len() {
        out[i] = plug[core[i][plug[ct[i] as usize] as usize] as usize];
    }
}

/// Encrypt or decrypt (Enigma is its own inverse). Input/output are 0..26.
pub fn run(s: &Settings, text: &[u8]) -> Vec<u8> {
    let core = core_perms(s, text.len());
    let mut out = vec![0u8; text.len()];
    apply(text, &core, &s.plug, &mut out);
    out
}

pub fn rotor_index(name: &str) -> Option<usize> {
    ROTORS.iter().position(|r| r.0.eq_ignore_ascii_case(name))
}

pub fn greek_index(name: &str) -> Option<usize> {
    GREEK.iter().position(|g| g.0.eq_ignore_ascii_case(name))
}

pub fn thin_index(name: &str) -> Option<usize> {
    let n = name.trim_start_matches("UKW-").trim_start_matches("UKW");
    THIN.iter().position(|r| r.0.eq_ignore_ascii_case(n))
}

pub fn reflector_index(name: &str) -> Option<usize> {
    let n = name.trim_start_matches("UKW-").trim_start_matches("UKW");
    REFLECTORS.iter().position(|r| r.0.eq_ignore_ascii_case(n))
}

pub fn to_letters(s: &str) -> Vec<u8> {
    s.bytes()
        .filter(|b| b.is_ascii_alphabetic())
        .map(|b| b.to_ascii_uppercase() - b'A')
        .collect()
}

pub fn to_string(v: &[u8]) -> String {
    v.iter().map(|&c| (c + b'A') as char).collect()
}

/// Group text into blocks of `n` letters, as Enigma messages were sent.
pub fn grouped(v: &[u8], n: usize) -> String {
    to_string(v)
        .as_bytes()
        .chunks(n)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn plug_pairs(plug: &[u8; 26]) -> String {
    let mut pairs = vec![];
    for i in 0..26 {
        let j = plug[i] as usize;
        if j > i {
            pairs.push(format!("{}{}", (b'A' + i as u8) as char, (b'A' + j as u8) as char));
        }
    }
    pairs.join(" ")
}

pub fn parse_plugs(s: &str) -> Result<[u8; 26], String> {
    let mut plug = identity_plug();
    for pair in s.split(|c: char| c == ' ' || c == ',').filter(|p| !p.is_empty()) {
        let l = to_letters(pair);
        if l.len() != 2 || l[0] == l[1] {
            return Err(format!("bad plug pair '{pair}'"));
        }
        let (a, b) = (l[0] as usize, l[1] as usize);
        if plug[a] != a as u8 || plug[b] != b as u8 {
            return Err(format!("letter used twice in plugboard: '{pair}'"));
        }
        plug[a] = b as u8;
        plug[b] = a as u8;
    }
    Ok(plug)
}

/// Parse `n` letters ("BUL") or 1-based numbers ("02 21 12", "2,21,12"),
/// e.g. ring settings or start positions (n = 4 for an M4).
pub fn parse_letters(s: &str, n: usize) -> Result<Vec<u8>, String> {
    let nums: Vec<&str> = s
        .split(|c: char| c == ' ' || c == ',' || c == '-')
        .filter(|p| !p.is_empty())
        .collect();
    if nums.len() == n && nums.iter().all(|n| n.chars().all(|c| c.is_ascii_digit())) {
        let mut out = vec![0u8; n];
        for (i, n) in nums.iter().enumerate() {
            let v: u8 = n.parse().map_err(|_| format!("bad number '{n}'"))?;
            if !(1..=26).contains(&v) {
                return Err(format!("ring/position number {v} not in 1..26"));
            }
            out[i] = v - 1;
        }
        return Ok(out);
    }
    let l = to_letters(s);
    if l.len() != n {
        return Err(format!("expected {n} letters or numbers, got '{s}'"));
    }
    Ok(l)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(rotors: &str, refl: &str, rings: &str, pos: &str, plugs: &str) -> Settings {
        Settings {
            rotors: std::array::from_fn(|i| {
                rotor_index(rotors.split(',').nth(i).unwrap()).unwrap()
            }),
            reflector: Refl::Std(reflector_index(refl).unwrap() as u8),
            rings: parse_letters(rings, 3).unwrap().try_into().unwrap(),
            pos: parse_letters(pos, 3).unwrap().try_into().unwrap(),
            plug: parse_plugs(plugs).unwrap(),
        }
    }

    #[test]
    fn classic_test_vector() {
        let s = settings("I,II,III", "B", "AAA", "AAA", "");
        assert_eq!(to_string(&run(&s, &to_letters("AAAAA"))), "BDZGO");
    }

    #[test]
    fn double_step() {
        // Rotor II at E triggers the double step of the middle rotor.
        let rot: [Rotor; 3] = std::array::from_fn(|i| Rotor::new(i));
        let mut pos = [0, 3, 20]; // A D U
        let mut seen = vec![];
        for _ in 0..4 {
            step(&mut pos, &rot[1].notch, &rot[2].notch);
            seen.push(to_string(&pos));
        }
        assert_eq!(seen, ["ADV", "AEW", "BFX", "BFY"]);
    }

    #[test]
    fn barbarossa_1941() {
        // Operation Barbarossa message, 7 July 1941 (part 1), as published
        // by Frode Weierud / Wikipedia.
        let s = settings("II,IV,V", "B", "02 21 12", "BLA", "AV BS CG DL FU HZ IN KM OW RX");
        let ct = "EDPUDNRGYSZRCXNUYTPOMRMBOFKTBZREZKMLXLVEFGUEYSIOZVEQMIKUBPMMYLKLTTDEIS\
                  MDICAGYKUACTCDOMOHWXMUUIAUBSTSLRNBZSZWNRFXWFYSSXJZVIJHIDISHPRKLKAYUPADT\
                  XQSPINQMATLPIFSVKDASCTACDPBOPVHJK";
        let pt = to_string(&run(&s, &to_letters(ct)));
        assert!(pt.starts_with("AUFKLXABTEILUNGXVONXKURTINOWAXKURTINOWAX"), "{pt}");
    }

    #[test]
    fn m4_compatibility_mode() {
        // Beta at A + thin B is wired to equal the M3 reflector B, and
        // Gamma at A + thin C equals reflector C.
        let b = Refl::M4 { greek: 0, thin: 0, off: 0 }.wiring();
        assert_eq!(b, Refl::Std(1).wiring());
        let c = Refl::M4 { greek: 1, thin: 1, off: 0 }.wiring();
        assert_eq!(c, Refl::Std(2).wiring());
        for off in 0..26 {
            let w = Refl::M4 { greek: 1, thin: 0, off }.wiring();
            assert!((0..26).all(|i| w[w[i] as usize] as usize == i && w[i] as usize != i));
        }
    }

    #[test]
    fn m4_u264_1942() {
        // U-264 (Kapitaenleutnant Hartwig Looks), 25 November 1942, broken by
        // the M4 Message Breaking Project in 2006.
        // UKW thin B, Beta II IV I, rings A A A V, start V J N A.
        let mut s = settings("II,IV,I", "B", "AAV", "JNA", "AT BL DF GJ HM NW OP QY RZ VX");
        s.reflector = Refl::M4 { greek: 0, thin: 0, off: (b'V' - b'A') };
        let ct = "NCZWVUSXPNYMINHZXMQXSFWXWLKJAHSHNMCOCCAKUQPMKCSMHKSEINJUSBLKIOSXCKUBHMLLXCSJUSRRDVKOHULXWCCBGVLIYXEOAHXRHKKFVDREWEZLXOBAFGYUJQUKGRTVUKAMEURBVEKSUHHVOYHABCJWMAKLFKLMYFVNRIZRVVRTKOFDANJMOLBGFFLEOPRGTFLVRHOWOPBEKVWMUQFMPWPARMFHAGKXIIBG";
        let pt = to_string(&run(&s, &to_letters(ct)));
        assert!(pt.starts_with("VONVONJLOOKSJHFFTTTEINSEINSDREIZWOYYQNNSNEUNINHALTXXBEIANGRIFFUNTERWASSERGEDRUECKT"), "{pt}");
    }

    #[test]
    fn self_inverse() {
        let s = settings("VI,VIII,III", "C", "XQD", "ZMA", "AB CD EF GH");
        let msg = to_letters("DERFEINDSTEHTVORDENTOREN");
        assert_eq!(run(&s, &run(&s, &msg)), msg);
    }
}
