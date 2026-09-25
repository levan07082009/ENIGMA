//! Language statistics used to recognise correct plaintext.
//! Scores are integer log-probabilities (log10 * 1000) so they are cheap to
//! sum in the hot loops.

pub struct Lang {
    pub name: &'static str,
    pub bi: Vec<i32>,
    pub tri: Vec<i32>,
    /// Average trigram score per letter of real text in this language.
    pub typical: f64,
    /// Average trigram score per letter of uniformly random letters.
    pub random: f64,
}

impl Lang {
    pub fn german() -> Lang {
        Lang::parse("German", include_str!("../data/german.txt"))
    }

    pub fn english() -> Lang {
        Lang::parse("English", include_str!("../data/english.txt"))
    }

    pub fn by_name(name: &str) -> Option<Lang> {
        match name.to_ascii_lowercase().as_str() {
            "de" | "german" | "deutsch" => Some(Lang::german()),
            "en" | "english" => Some(Lang::english()),
            _ => None,
        }
    }

    fn parse(name: &'static str, data: &str) -> Lang {
        let mut bi = vec![0f64; 26 * 26];
        let mut tri = vec![0f64; 26 * 26 * 26];
        for line in data.lines().filter(|l| !l.starts_with('#')) {
            let (gram, count) = line.split_once(' ').unwrap();
            let count: f64 = count.parse().unwrap();
            let idx = gram.bytes().fold(0usize, |a, b| a * 26 + (b - b'A') as usize);
            if gram.len() == 2 { bi[idx] = count } else { tri[idx] = count }
        }
        let to_log = |t: &[f64]| -> (Vec<i32>, f64, f64) {
            let total: f64 = t.iter().sum();
            let floor = (0.01 / total).log10();
            let logs: Vec<f64> = t
                .iter()
                .map(|&c| if c > 0.0 { (c / total).log10() } else { floor })
                .collect();
            let typical = t.iter().zip(&logs).map(|(c, l)| c / total * l).sum::<f64>();
            let random = logs.iter().sum::<f64>() / logs.len() as f64;
            (logs.iter().map(|l| (l * 1000.0).round() as i32).collect(), typical, random)
        };
        let (bi, _, _) = to_log(&bi);
        let (tri, typical, random) = to_log(&tri);
        Lang { name, bi, tri, typical, random }
    }

    #[inline(always)]
    pub fn trigram_score(&self, pt: &[u8]) -> i32 {
        let mut s = 0;
        for w in pt.windows(3) {
            s += self.tri[w[0] as usize * 676 + w[1] as usize * 26 + w[2] as usize];
        }
        s
    }

    #[inline(always)]
    pub fn bigram_score(&self, pt: &[u8]) -> i32 {
        let mut s = 0;
        for w in pt.windows(2) {
            s += self.bi[w[0] as usize * 26 + w[1] as usize];
        }
        s
    }

    /// Trigram score per trigram, in log10 units, comparable to
    /// `typical` / `random`.
    pub fn per_letter(&self, pt: &[u8]) -> f64 {
        if pt.len() < 3 {
            return self.random;
        }
        self.trigram_score(pt) as f64 / 1000.0 / (pt.len() - 2) as f64
    }
}

/// Sum of n(n-1) over letter counts: proportional to the index of
/// coincidence for a fixed length.
#[inline(always)]
pub fn ioc_score(pt: &[u8]) -> i32 {
    let mut counts = [0i32; 26];
    for &c in pt {
        counts[c as usize] += 1;
    }
    counts.iter().map(|&n| n * (n - 1)).sum()
}

pub fn ioc(pt: &[u8]) -> f64 {
    let n = pt.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    ioc_score(pt) as f64 / (n * (n - 1.0))
}
