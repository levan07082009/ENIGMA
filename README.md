# Enigma cracker

A fast, dependency-free Enigma I / M3 / **M4** simulator and
**ciphertext-only** code breaker written in Rust. It uses every CPU core.

* Machines: the 3-rotor Enigma I / M3, and the 4-rotor naval **M4**
  (Greek wheels Beta/Gamma with thin reflectors B/C). All have rotors
  I–VIII, reflectors (UKW) A/B/C, ring settings, plugboard, and the
  middle-rotor double step. The simulator is verified against two real
  wartime messages (`cargo test`): the M3 *Operation Barbarossa* message
  of 7 July 1941, and the M4 message to U-264 of 25 November 1942.
* Attack: an exhaustive rotor/ring search, then a plugboard hill-climb scored
  with German and English n-gram statistics.

**No Rust on your computer?** Open [`enigma_colab.ipynb`](enigma_colab.ipynb)
in [Google Colab](https://colab.research.google.com/) (File → Open notebook
→ GitHub). It installs everything and walks through each command.

## Build

Install Rust from <https://rustup.rs>, then:

```sh
cargo build --release          # binary: target/release/enigma
cargo test --release           # machine + attack self-tests
```

## Usage

```sh
# 1. Is it even Enigma? (flat letter statistics, length, verdict)
enigma analyze -f ciphertext.txt

# 2. Crack it (default: Enigma I rotors I-V, reflector B, German+English)
enigma crack -f ciphertext.txt

#    widest 3-rotor search: naval rotors I-VIII, reflectors B and C
enigma crack -f ciphertext.txt --rotors all --reflector BC

#    naval 4-rotor M4: rotors I-VIII, Greek wheels Beta+Gamma, thin B+C
enigma crack -f ciphertext.txt --machine m4 --reflector BC

#    split a long search into 4 parts (run on different machines/sessions)
enigma crack -f ciphertext.txt --machine m4 --reflector BC --part 1/4

#    you think there is no plugboard: skip the plugboard climb
enigma crack -f ciphertext.txt --max-plugs 0

# 3. Got a guess for a word in the message (a "crib")? See where it can sit:
enigma crib --crib WETTERBERICHT -f ciphertext.txt

# 4. Decrypt / encrypt with known settings (Enigma is symmetric)
enigma run --rotors II,IV,V --reflector B --rings BUL --start BLA \
           --plugs "AV BS CG DL FU HZ IN KM OW RX" -f ciphertext.txt
#    M4: put the Greek wheel first, give 4 ring and start letters
enigma run --rotors Beta,II,IV,I --reflector B --rings AAAV --start VJNA \
           --plugs "AT BL DF GJ HM NW OP QY RZ VX" -f message.txt

# 5. See it work on a random key
enigma demo --length 200 --plugs 0
```

Ciphertext can be given as arguments, with `-f FILE`, or on stdin. Spaces
and punctuation are ignored.

Results print the key in the historic terms: Walzenlage (rotor order),
Ringstellung (rings), Grundstellung (start position) and Stecker
(plugboard). **Fitness** measures how language-like the plaintext is:
0 means random letters and 1 means typical text. Real solutions usually
score above 0.75.

Only the middle and right rings affect the output. The left ring just
shifts the left rotor's start position, so the tool reports it as `A`
(and likewise the Greek wheel's ring on an M4). The key you get is
*equivalent* to the original, not necessarily identical.

## How the attack works

The key space is about 10^23. Nearly all of that comes from the plugboard,
so the attack splits the problem:

1. **Rotor search (exhaustive).** Every rotor order × reflector × 26³ start
   positions × every distinct stepping pattern the ring settings can cause
   is decrypted with an *empty* plugboard. For short messages that is about
   a billion trial decryptions for rotors I–V. Each trial is scored with
   trigram log-probabilities (German and English) and with the index of
   coincidence. The best few thousand from each list are kept. The left
   rotor's position never changes the stepping, so each stepping pattern
   is computed once and reused across the 26 left positions.
2. **Plugboard hill-climb.** For each kept candidate, cables are greedily
   added, removed and re-routed. The climb is scored first by IoC, then
   bigrams, then trigrams, and repeats until nothing improves.
3. **Ring refinement.** All 676 middle/right ring settings are tried with
   the recovered plugboard, and the plugboard is re-climbed if the rings
   changed. Each extra cable must improve the score by a fixed margin, so
   short texts don't collect bogus cables.

**M4 trick.** The M4's Greek wheel never steps. Greek wheel plus thin
reflector therefore act as one fixed reflector, so an M4 is exactly a
3-rotor machine with one of 104 reflectors: Beta/Gamma × thin B/C × 26
wheel positions. The same attack runs unchanged, 52× larger than a
two-reflector M3 search. For M4 the ring search defaults to the fast mode:
right-ring stepping in phase 1, then the full refinement in phase 2.

## What can realistically be broken

Measured with `enigma demo` on random keys, 3 random keys per row (seeds 11–13). It
used German text, rotors I–V, reflector B and 4 CPU cores. *Readable*
means ≥ 90% of letters recovered; the misses are typically the last few
letters after a rotor step that the short text could not pin down.

| Length | Plug cables | Result | Time per crack |
|---|---|---|---|
| 72 | 0 | 3/3 readable (68–69 of 72 letters) | ~25 s |
| 72 | 3 | 1 exact, 2 readable (66 of 72) | ~25 s |
| 150 | 0 | 1 exact, 2 readable (135 of 150) | ~1 min |
| 150 | 5 | 3/3 exact | ~1 min |
| 150 | 10 | **0/3** | ~1 min |
| 239 | 10 | **0/3** | ~1 min |

With all eight rotors and both reflectors, the search is about 11× larger.
An M4 demo restricted to rotors I–III and Beta (100 letters, 3 cables)
recovered the message exactly in 17 s. The full M4 search on a 72-letter
message takes about 30 min on 4 cores, or about an hour on free Colab.

Why a full plugboard is hard: with 10 cables, 20 of the 26 letters are
swapped. The rotor-only decryption at the correct setting is then right for
only about a quarter of the letters, and that signal drowns among the 10^7+
wrong settings. The Allies needed cribs (guessed plaintext) and the Bombe
for exactly this reason. The published modern ciphertext-only attacks
(Weierud & Sullivan 2005, Ostwald & Weierud 2017) hill-climb the plugboard
for *every* rotor setting. That is a few hundred times more compute than
this tool's default, and it is the natural next step.

## Recommendations: fastest way to break an unknown message

1. **Confirm it is Enigma.** Run `enigma analyze`. An IoC near 0.038 fits
   Enigma. An IoC near 0.066–0.076 means a simple cipher, not Enigma.
2. **Use context.** Where was the message found? Puzzles and challenges
   often use the default settings, no plugboard, or give hints (date,
   rotor order, a key sheet). Every fact you pin down (for example
   `--rotors I,II,III`) shrinks the search.
3. **Run the exhaustive search**: `enigma crack --rotors all --reflector BC`.
   If there is no plugboard or only a few cables, this finds it.
4. **Look for a crib.** German military messages often contained
   `WETTER`, `KEINEBESONDERENEREIGNISSE`, `ANX` (to...), `OBERKOMMANDO`, a
   unit name, or a sign-off. Because Enigma never encrypts a letter to
   itself, `enigma crib` shows where a guessed word can sit. A crib plus
   a Bombe-style search breaks short, fully plugged messages. That is the
   natural next feature for this tool.
5. **Get more ciphertext.** Several messages on the same day share rotors,
   rings and plugboard. Together they add up to enough text for the
   statistical attack.

## The message in `ciphertext.txt`: U-534, 1 May 1945

```
JCRSA JTGSJ EYEXY KKZZS HVUOC TRFRC RPFVY PLKPP LGRHV VBBTB RSXSW XGGTY TVKQN GSCHV GF
```

This is **P1030680**, the best-known *unbroken* German naval Enigma message.
The U-boat U-534 received it on 1 May 1945. It is 72 letters long, with
indicator groups `VROL NMKA`. Its facts
([enigma.hoerenberg.com](https://enigma.hoerenberg.com/index.php?cat=Unbroken)):

* **Machine: Enigma M4** (four rotors: the naval rotors plus a thin
  "Greek" wheel Beta/Gamma and a thin reflector B/C). All settings are
  unknown.
* The message is believed to use the key **"Thetis"**, which the boat did
  not have on board. The radio operator's own two attempts produced
  gibberish.
* The distributed [Enigma@Home](https://www.enigmaathome.net/) project has
  been attacking it for years without success.

What this tool established (`--rotors all --reflector BC --rings full`,
about 3 minutes on 4 cores):

* With **no plugboard**, no three-rotor Enigma I/M3 setting (rotors
  I–VIII, reflector B or C, all positions and ring settings) decrypts it
  to readable German or English. The search is exhaustive, so that case
  is ruled out.
* With the plugboard climb enabled, the top results score high but are
  gibberish. That is overfitting: 72 letters are too few to pin down 10
  cables statistically.

The full **M4** search (`--machine m4 --reflector BC`) checks all 336
rotor orders × Beta/Gamma × thin B/C × every Greek-wheel and start
position. It took 28 minutes on 4 cores. The result is the same: with no
plugboard, no M4 setting decrypts the message to readable text. With a
few cables that would very likely have shown up too. The top-ranked
results are again statistics-shaped gibberish (for example
`EATHIGHOMEOWFRE...`).

This fits the history: naval keys used 10 plugboard cables, and at 72
letters that is beyond any ciphertext-only attack (see the table above).
Breaking it realistically needs either a crib or a very large compute
budget, and Enigma@Home's volunteer grid has already spent the latter.
The most promising angles are historical: the Thetis key's properties,
the indicator `VROL NMKA`, and likely cribs for a message sent to a
U-boat on 1 May 1945. `enigma crib` shows where a guessed word can sit.
