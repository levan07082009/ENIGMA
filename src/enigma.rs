//! Enigma I / M3 machine model: rotors I-VIII, reflectors A/B/C, ring
//! settings, plugboard and the double-stepping anomaly.

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

pub fn reflector(index: usize) -> [u8; 26] {
    let mut r = [0u8; 26];
    for (i, b) in REFLECTORS[index].1.bytes().enumerate() {
        r[i] = b - b'A';
    }
    r
}

/// A complete daily key + message start position.
/// Arrays are ordered left, middle, right.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub rotors: [usize; 3],
    pub reflector: usize,
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
    let refl = reflector(s.reflector);
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

/// Accepts "BUL", "B U L" or "02 21 12" / "2,21,12" (1-based numbers).
pub fn parse_triple(s: &str) -> Result<[u8; 3], String> {
    let nums: Vec<&str> = s
        .split(|c: char| c == ' ' || c == ',' || c == '-')
        .filter(|p| !p.is_empty())
        .collect();
    if nums.len() == 3 && nums.iter().all(|n| n.chars().all(|c| c.is_ascii_digit())) {
        let mut out = [0u8; 3];
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
    if l.len() != 3 {
        return Err(format!("expected three letters or numbers, got '{s}'"));
    }
    Ok([l[0], l[1], l[2]])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(rotors: &str, refl: &str, rings: &str, pos: &str, plugs: &str) -> Settings {
        Settings {
            rotors: std::array::from_fn(|i| {
                rotor_index(rotors.split(',').nth(i).unwrap()).unwrap()
            }),
            reflector: reflector_index(refl).unwrap(),
            rings: parse_triple(rings).unwrap(),
            pos: parse_triple(pos).unwrap(),
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
    fn self_inverse() {
        let s = settings("VI,VIII,III", "C", "XQD", "ZMA", "AB CD EF GH");
        let msg = to_letters("DERFEINDSTEHTVORDENTOREN");
        assert_eq!(run(&s, &run(&s, &msg)), msg);
    }
}
