mod crack;
mod enigma;
mod stats;

use crack::{Options, crack, crib_positions};
use enigma::*;
use stats::{Lang, ioc};
use std::io::Read;
use std::time::Instant;

const USAGE: &str = "\
Enigma I / M3 simulator and ciphertext-only cracker

USAGE:
  enigma crack   [OPTIONS] [CIPHERTEXT...]   break a message (text, -f FILE or stdin)
  enigma analyze [CIPHERTEXT...]             statistics: is this plausibly Enigma?
  enigma crib    --crib WORD [CIPHERTEXT...] where can a known word sit?
  enigma run     --rotors II,IV,V --reflector B --rings BUL --start BLA
                 --plugs \"AV BS CG\" [TEXT...]   encrypt = decrypt
  enigma demo    [--length N] [--plugs N] [--lang de|en] [--seed N]

CRACK OPTIONS:
  --lang de|en|auto     plaintext language model          (default auto = both)
  --rotors SET          I-V (Enigma I / army), all (I-VIII, naval M3),
                        or a list such as I,II,IV         (default I-V)
  --reflector SET       B, C, A or a combination like BC  (default B)
  --max-plugs N         plugboard cables to search, 0-13  (default 10)
  --rings auto|full|fast  full = also brute-force ring settings in phase 1
                        (slower, needed for short messages); auto = full
                        when the message has <= 200 letters (default auto)
  --keep N              phase-1 candidates kept per metric (default 2000)
  --threads N           worker threads                    (default all cores)
  --show N              number of solutions printed       (default 3)
  -f FILE               read ciphertext from a file
";

struct Args {
    flags: Vec<(String, String)>,
    rest: Vec<String>,
}

impl Args {
    fn parse(raw: &[String]) -> Args {
        let mut flags = vec![];
        let mut rest = vec![];
        let mut i = 0;
        while i < raw.len() {
            let a = &raw[i];
            if a.starts_with('-') && a.len() > 1 && !a[1..].chars().all(|c| c.is_ascii_digit()) {
                let val = raw.get(i + 1).cloned().unwrap_or_default();
                flags.push((a.trim_start_matches('-').to_string(), val));
                i += 2;
            } else {
                rest.push(a.clone());
                i += 1;
            }
        }
        Args { flags, rest }
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.flags.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn num(&self, name: &str, default: usize) -> usize {
        self.get(name).map_or(default, |v| v.parse().unwrap_or_else(|_| die(&format!("--{name} needs a number"))))
    }

    fn text(&self) -> Vec<u8> {
        let raw = if let Some(path) = self.get("f") {
            std::fs::read_to_string(path).unwrap_or_else(|e| die(&format!("{path}: {e}")))
        } else if !self.rest.is_empty() {
            self.rest.join(" ")
        } else {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s).ok();
            s
        };
        let t = to_letters(&raw);
        if t.is_empty() {
            die("no letters A-Z in the input");
        }
        t
    }
}

fn die(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(2)
}

fn parse_rotor_set(s: &str) -> Vec<usize> {
    match s.to_ascii_lowercase().as_str() {
        "i-v" | "army" | "5" => (0..5).collect(),
        "all" | "i-viii" | "naval" | "8" => (0..8).collect(),
        _ => s
            .split(',')
            .map(|r| rotor_index(r.trim()).unwrap_or_else(|| die(&format!("unknown rotor '{r}'"))))
            .collect(),
    }
}

fn parse_reflector_set(s: &str) -> Vec<usize> {
    s.split(|c: char| c == ',' || c == ' ')
        .flat_map(|p| p.chars().map(|c| c.to_string()).collect::<Vec<_>>())
        .filter(|p| !p.trim().is_empty())
        .map(|r| reflector_index(&r).unwrap_or_else(|| die(&format!("unknown reflector '{r}'"))))
        .collect()
}

fn describe(s: &Settings) -> String {
    let names: Vec<&str> = s.rotors.iter().map(|&r| ROTORS[r].0).collect();
    let nums: Vec<String> = s.rings.iter().map(|r| format!("{:02}", r + 1)).collect();
    let plugs = plug_pairs(&s.plug);
    format!(
        "  Reflector (UKW):        {}\n  \
           Rotors (Walzenlage):    {}\n  \
           Rings (Ringstellung):   {} ({})\n  \
           Start (Grundstellung):  {}\n  \
           Plugboard (Stecker):    {}",
        REFLECTORS[s.reflector].0,
        names.join(" "),
        to_string(&s.rings),
        nums.join(" "),
        to_string(&s.pos),
        if plugs.is_empty() { "(none)".into() } else { plugs }
    )
}

fn settings_from_args(a: &Args) -> Settings {
    let rotors = parse_rotor_set(a.get("rotors").unwrap_or_else(|| die("--rotors required, e.g. II,IV,V")));
    if rotors.len() != 3 {
        die("--rotors needs exactly three rotors, left to right");
    }
    let refl = parse_reflector_set(a.get("reflector").unwrap_or("B"));
    let plugs = parse_plugs(a.get("plugs").unwrap_or("")).unwrap_or_else(|e| die(&e));
    Settings {
        rotors: [rotors[0], rotors[1], rotors[2]],
        reflector: refl[0],
        rings: parse_triple(a.get("rings").unwrap_or("AAA")).unwrap_or_else(|e| die(&e)),
        pos: parse_triple(a.get("start").unwrap_or("AAA")).unwrap_or_else(|e| die(&e)),
        plug: plugs,
    }
}

fn analyze(ct: &[u8]) {
    let n = ct.len();
    let ic = ioc(ct);
    let mut counts = [0usize; 26];
    for &c in ct {
        counts[c as usize] += 1;
    }
    println!("Length:               {n} letters");
    println!("Index of coincidence: {ic:.4}   (random/Enigma ≈ 0.0385, German ≈ 0.076, English ≈ 0.066)");
    print!("Letter counts:        ");
    for (i, c) in counts.iter().enumerate() {
        print!("{}{} ", (b'A' + i as u8) as char, c);
    }
    println!();
    let missing: String = (0..26).filter(|&i| counts[i] == 0).map(|i| (b'A' + i as u8) as char).collect();
    println!("Letters never used:   {}", if missing.is_empty() { "-".into() } else { missing });
    println!();
    // Standard error of the IoC of random text is roughly sqrt(2/(26 n^2)).
    let sigma = (2.0f64 / 26.0).sqrt() / n as f64;
    let z = (ic - 1.0 / 26.0) / sigma;
    if z > 4.0 {
        println!(
            "Verdict: IoC is {z:.1} standard deviations above random. Enigma output is\n\
             essentially flat, so this looks more like a simple substitution,\n\
             transposition or Vigenere-style cipher than Enigma."
        );
    } else {
        println!("Verdict: letter statistics are flat, consistent with Enigma (or another\npolyalphabetic cipher).");
    }
    if n < 100 {
        println!(
            "Note: {n} letters is short. Without a plugboard (or with only a few\n\
             cables) it is usually breakable; with a full 10-cable plugboard,\n\
             ciphertext-only attacks become unreliable below ~100-150 letters.\n\
             A crib (known word) helps enormously."
        );
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

pub const DEMO_DE: &str = "ANXOBERKOMMANDODERWEHRMACHTXVORMARSCHDERPANZERGRUPPEZWEIHATPLANMAESSIG\
BEGONNENXFEINDLICHEKRAEFTEWERDENBEIDERSTADTERWARTETXWETTERBERICHTFUERDIENACHTXLEICHTERREGEN\
AUSWESTENXKEINEBESONDERENEREIGNISSEXERBITTEWEITEREBEFEHLEXGENERALSTABDESHEERES";
const DEMO_EN: &str = "THEATTACKWILLBEGINATDAWNFROMTHENORTHERNRIDGEXREINFORCEMENTSAREEXPECTED\
BEFORENOONXTHEWEATHERREPORTFORTHENIGHTISLIGHTRAINFROMTHEWESTXNOSPECIALEVENTSTOREPORTXAWAIT\
FURTHERORDERSFROMHEADQUARTERS";

fn options(a: &Args, n: usize) -> Options {
    let langs = match a.get("lang").unwrap_or("auto") {
        "auto" | "both" => vec![Lang::german(), Lang::english()],
        l => vec![Lang::by_name(l).unwrap_or_else(|| die(&format!("unknown language '{l}'")))],
    };
    Options {
        rotors: parse_rotor_set(a.get("rotors").unwrap_or("I-V")),
        reflectors: parse_reflector_set(a.get("reflector").unwrap_or("B")),
        langs,
        keep: a.num("keep", 2000),
        max_plugs: a.num("max-plugs", 10).min(13),
        full_rings: match a.get("rings").unwrap_or("auto") {
            "full" => true,
            "fast" => false,
            _ => n <= 200,
        },
        threads: a.num(
            "threads",
            std::thread::available_parallelism().map_or(4, |n| n.get()),
        ),
        progress: true,
    }
}

fn run_crack(ct: &[u8], opts: &Options, show: usize) -> Vec<crack::Solution> {
    let orders = opts.rotors.len() * (opts.rotors.len() - 1) * (opts.rotors.len() - 2);
    eprintln!(
        "Cracking {} letters: {} rotor orders x {} reflector(s), {} language model(s), up to {} plug cables, {} ring search, {} threads",
        ct.len(),
        orders,
        opts.reflectors.len(),
        opts.langs.len(),
        opts.max_plugs,
        if opts.full_rings { "full" } else { "fast" },
        opts.threads
    );
    let t = Instant::now();
    let sols = crack(ct, opts);
    eprintln!("Finished in {:.1}s\n", t.elapsed().as_secs_f64());
    for (i, s) in sols.iter().take(show).enumerate() {
        // A plugboard climb can bend short nonsense into language-like
        // letter statistics, so a high score only counts when there is
        // enough text for the number of cables used.
        let cables = plug_pairs(&s.settings.plug).split_whitespace().count();
        let trusted = cables == 0 || ct.len() >= 150;
        let verdict = match s.fitness {
            f if f > 0.75 && trusted => "very likely correct",
            f if f > 0.75 => "suspicious: short text + plugboard can fake this, check it reads as words",
            f if f > 0.55 => "probably wrong / at best partially correct",
            _ => "wrong",
        };
        println!(
            "#{}  fitness {:.2} ({verdict}), {}\n{}\n  Plaintext:\n    {}\n",
            i + 1,
            s.fitness,
            s.lang,
            describe(&s.settings),
            grouped(&s.plaintext, 5),
        );
    }
    println!("Fitness: 0 = reads like random letters, 1 = reads like typical text.");
    println!("Only trust a result whose plaintext actually reads as words.");
    sols
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = raw.first() else {
        print!("{USAGE}");
        return;
    };
    let a = Args::parse(&raw[1..]);
    match cmd.as_str() {
        "crack" => {
            let ct = a.text();
            run_crack(&ct, &options(&a, ct.len()), a.num("show", 3));
        }
        "analyze" | "analyse" => analyze(&a.text()),
        "crib" => {
            let ct = a.text();
            let crib = to_letters(a.get("crib").unwrap_or_else(|| die("--crib WORD required")));
            let pos = crib_positions(&ct, &crib);
            println!(
                "{} of {} positions are possible for crib {} (Enigma never encrypts a letter to itself):",
                pos.len(),
                (ct.len() + 1).saturating_sub(crib.len()),
                to_string(&crib)
            );
            for p in pos {
                println!("  offset {p:3}: {}  under  {}", to_string(&crib), to_string(&ct[p..p + crib.len()]));
            }
        }
        "run" | "encrypt" | "decrypt" => {
            let s = settings_from_args(&a);
            println!("{}", grouped(&run(&s, &a.text()), 5));
        }
        "demo" => {
            let seed = a.num("seed", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |d| d.as_nanos() as usize));
            let mut rng = Rng(seed as u64 | 1);
            let lang = a.get("lang").unwrap_or("de");
            let text = to_letters(if lang == "en" { DEMO_EN } else { DEMO_DE });
            let len = a.num("length", 150).min(text.len());
            let nplugs = a.num("plugs", 10).min(13);
            let mut rotors: Vec<usize> = (0..5).collect();
            for i in (1..5).rev() {
                rotors.swap(i, rng.below(i + 1));
            }
            let mut letters: Vec<usize> = (0..26).collect();
            for i in (1..26).rev() {
                letters.swap(i, rng.below(i + 1));
            }
            let mut plug = identity_plug();
            for k in 0..nplugs {
                plug[letters[2 * k]] = letters[2 * k + 1] as u8;
                plug[letters[2 * k + 1]] = letters[2 * k] as u8;
            }
            let s = Settings {
                rotors: [rotors[0], rotors[1], rotors[2]],
                reflector: 1,
                rings: std::array::from_fn(|_| rng.below(26) as u8),
                pos: std::array::from_fn(|_| rng.below(26) as u8),
                plug,
            };
            let pt = &text[..len];
            let ct = run(&s, pt);
            println!("Demo (seed {seed}) with a random secret key:\n{}\n", describe(&s));
            println!("Ciphertext:\n    {}\n", grouped(&ct, 5));
            let mut opts = options(&a, ct.len());
            opts.langs = vec![Lang::by_name(lang).unwrap_or_else(Lang::german)];
            let sols = run_crack(&ct, &opts, a.num("show", 1));
            let right = sols.first().map_or(0, |b| b.plaintext.iter().zip(pt).filter(|(a, b)| a == b).count());
            println!("Recovered {right}/{len} letters of the plaintext ({}).",
                match right * 100 / len.max(1) {
                    100 => "SUCCESS: exact",
                    90.. => "SUCCESS: readable",
                    _ => "FAILED",
                });
        }
        "help" | "-h" | "--help" => print!("{USAGE}"),
        _ => die(&format!("unknown command '{cmd}'\n\n{USAGE}")),
    }
}
