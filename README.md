# lnurlcash-core

LNURLcash ([LUD-25 draft](https://github.com/lnurl/luds/pull/301)) bearer
notes: the money-critical logic, in one place, with bindings for other
languages.

```toml
[dependencies]
lnurlcash-core = "0.2"
```

Early `0.x`, tracking a **draft** spec. Pin an exact version.

## Why a Rust core

The LNURLcash wire protocol is small enough to implement in an afternoon,
which is exactly the problem: the protocol is easy and the discipline is not.
Ambiguous mutations, melt semantics, who generates a replacement secret, which
end of a signature carries the recovery id — get any of those wrong and it
works perfectly until it costs somebody their money.

This crate exists so there is one audited implementation of that discipline,
and so the mobile bindings are a wrapper over it rather than a fifth
hand-written port drifting away from a draft spec.

## Layout

**`protocol`** has no I/O in it. Each operation is a `Request` — a URL to GET,
plus the fresh secrets that must survive if the answer is lost — paired with a
`parse_*` function for what comes back. A caller with its own HTTP stack needs
nothing else.

**`client`** (feature `client`) is a thin reqwest loop over exactly that. Off
by default: the core is pure, and a caller with its own stack should not
compile one it will not use.

**`ffi`** (feature `ffi`) exports the pure half through UniFFI, for Kotlin and
Swift. Deliberately no async across the boundary — see below.

## Usage

```rust
use lnurlcash_core::{protocol, verify_note_signature};

// no HTTP in the core: build, GET it yourself, parse
let request = protocol::note_info_request(note_url)?;
let body: serde_json::Value = your_http_get(&request.url)?;
let info = protocol::parse_note_info(&body, note_url, protocol::Policy::default())?;

println!("{} msat", info.max_withdrawable);
```

With the `client` feature:

```rust
use lnurlcash_core::client::{Client, NoteFate};

let client = Client::new();
let info = client.fetch_note_info(note_url).await?;
let fresh = client.rotate_note(&info.callback, &info.k1).await?;
```

## The four things that will cost you money

**1. Never let the service generate a replacement secret.** On rotate, split
and merge the *wallet* draws a fresh 32 bytes and discloses only
`sha256(secret)` as `p1`. A service-issued replacement has, structurally, been
seen by that service, so a "rotate" that accepts one closes no exposure at
all.

**2. A failed mutation is not a failure.** If a rotate times out, the service
may already have burned the input and minted the output, and the fresh secret
in your process is the only copy of that money.

```rust
match client.rotate_note(&callback, &k1).await {
    Ok(rotated) => { /* ... */ }
    Err(err) if err.is_ambiguous() => {
        save(err.new_secrets())?;                    // first. always.
        match client.probe_burned_note(&note_url).await {
            NoteFate::Live => {}      // nothing landed; the saved secrets are worthless
            NoteFate::Gone => {}      // the burn landed; the saved secrets ARE the note
            NoteFate::Unknown => {}   // no information - keep everything
        }
    }
    Err(err) => return Err(err),
}
```

`Error::RequestRefused` is the opposite and safe: nothing left the process.
`Error::is_ambiguous()` and `Error::is_definitive()` are the two questions
worth asking about any failure here.

A mutation whose answer the transport lost is re-sent rather than given up on.
LUD-25 requires a service to answer a byte-identical rotate, split or merge
with the success it already returned, so the retry usually completes the
operation and the caller never sees an error at all
(`ClientConfig::mutation_retries`, default 1). Never a melt, which carries
`pr`, is paid asynchronously and has no replay guarantee; and never a
definitive refusal, which is the service's considered answer.

**2b. A certificate proves issuance, not that your spend opens the note.**
A mint certifies every note with a `cs1` over its public key `Q` and the
amount. `Q` and the certificate are both public, so anyone can pair them with
a `ck1` whose signature opens nothing. `check_note` (and
`verify_note_signature`, its yes-or-no) needs the mint's domain for exactly
that reason: it checks the spend opens `Q` there as well as the certificate.
`check_note_url` reads everything off a certified note URL.

A `cp1` output is owed its `cs1` whatever the policy says: without it
`parse_mutation` raises `Error::Unverifiable`, which **carries the fresh
secrets**, because the mutation landed and the note is real. A bearer output
named by its hash is certified by a mint with a signer and may come back
uncertified from one without; `Policy { require_signatures: true, ..Policy::default() }`
refuses that. Hand `parse_mutation` the request's `outputs` so it knows which
kind it asked for.

`parse_note_info` refuses a `withdrawRequest` without a valid `mintPubkey`;
`Policy { require_mint_pubkey: false, ..Policy::default() }` admits a mint
that publishes none, knowing nothing it issues can then be checked offline.

**3. A melt's `OK` means "in flight", not "spent".** The service pays
asynchronously and only burns the note once the payment settles, restoring it
if the payment fails. A failed melt is never reported back through the
callback — only observed as the note becoming spendable again.
`Error::NotePending` means retry, never spent.

**4. Rotate the instant you claim a minted note.** The preimage that mints a
note is generated by the service, and if it serves LUD-21 `verify`, anyone who
saw the unpaid invoice can poll for it. First rotater wins.

## Bindings

```bash
cargo build --release --features ffi
cargo run --features bindgen --bin uniffi-bindgen -- \
  generate --library target/release/liblnurlcash_core.dylib \
  --language kotlin --out-dir bindings/kotlin
```

Swap `--language swift` for iOS.

Only the pure half crosses the boundary: request building, response parsing,
signature verification, note URLs, fee arithmetic. HTTP stays on the other
side, where a mobile app already has a stack it trusts and a concurrency model
it likes. Async over FFI is where these bindings usually turn painful, and
there is nothing to gain from it — the interesting part of LNURLcash is not the
GET, it is knowing what a response means and which secrets must survive a
failure. Both of those are pure functions.

[lnurlcash-kotlin](https://github.com/lnurlcash/lnurlcash-kotlin) wraps
the generated Kotlin in something idiomatic.

## Two things ports get wrong

Both are caught by the conformance vectors, and both are worth knowing if you
are porting this anywhere:

**The proportional fee term overflows.** `gross * ppm / 1_000_000` exceeds
`u64::MAX` at realistic amounts — 21M BTC is 2.1e15 msat, times 999_999 ppm is
about 2.1e21. It is computed split. A naive version passes every small test.

**Gross-up must be a binary search.** Estimate-then-walk is unbounded at a
99.9999% fee — roughly a million steps — so any guard on it returns a
non-minimal answer, and the *service* picks the fee.

## Every note is a taproot output key

LUD-25 (as of lnurl/luds `6e865b1`) makes every note a BIP-341 output key
`Q`, written `cp1<Q>`. A mint stores, burns and certifies it by `hex(Q)`, and
a spend opens it the way a taproot output is spent on chain:

```text
64 hex          a bearer note's preimage: the short form of its cw1
ck1<Q || sig>   key path: a BIP-340 signature by Q
cw1<...>        script path: a leaf, its control block, and its witness
```

A bearer note is the one-leaf hashlock `OP_SHA256 <h> OP_EQUAL` under
BIP-341's NUMS key, so it has no key path. Everything but the preimage follows
from `h`, which is why the bearer wire looks as it always did: 64 hex where a
`k1` goes is the preimage, and 64 hex where a `cp1` goes is `h`. Only its id
moved, from `sha256(k1)` to `hex(Q)`. `note_id_of(k1)` gives it for any spend.

Every signature signs the BIP-341 sighash of one canonical, never-broadcast
transaction whose prevout is `tagged_hash("LNURLcash/mint", domain)`, so a
spend one mint has seen can never be replayed at another. `domain` is the
mint's lowercase hostname, no scheme, no port: `spend_domain_of(note_url)`.
The transaction's shape never changes, so the sighash is built field by field
in `spend` with no Bitcoin library, graded against every intermediate of spec
vector 3.

`check_spend(k1, domain)` says whether a spend opens its note as a mint would
judge it: `Opens`, `OpensLegacy` (a `ck1` under a deprecated scheme mints
still accept: rotate it), `Unevaluated` (a script this crate does not run;
only the bearer hashlock is evaluated) or `Fails`. Time claims are the mint's
clock to judge; `check_time_claim` and `check_leaf` are the mint's rules, for
telling a holder what a mint will do.

## Seed-recoverable notes

LUD-25 specifies one seed derivation, and this crate implements it:

```
cashHashingKey   = m/139'/0
(d1, d2, d3, d4) = HMAC-SHA256(cashHashingKey, host)[0..16] as 4 uint32
domainNode       = m/139'/d1/d2/d3/d4
```

`d1..d4` are used **exactly as they fall**. BIP-32 reads any index `>= 2^31`
as hardened, so which of the four levels are hardened is decided by the mint's
own host name. Masking the top bit, or hardening all four, derives a different
tree and restores nothing, silently.

Bearer notes are not derived from the seed. The wallet draws plain
randomness, so a bearer note is only as recoverable as the wallet's backup of
its preimage. Seed recovery is what key-path notes are for: their keys hang
off this same domain node (see below).

`derive_cash_domain_node` is its own step because it is the unit a signer is
provisioned with. Whoever holds it can derive every note key held at that
mint: provisioning material, one mint's subtree, not the wallet.

`secrets::derive_note_root` / `derive_note_secret` are the pre-spec HMAC
scheme this project shipped before the draft had one. Not deprecated, because
notes minted under it are still money; just not what to mint under.

**The counter is half the backup.** A SERVICE must answer a lookup for a
burned note exactly as it answers one for a note it never issued, and a rotate
burns the index below, so a wallet that has rotated more than its gap limit
cannot find its own position by scanning. The per-host counter is not secret
(an index reveals nothing without the root), so back it up, and merge it
upwards only.

## Key-path notes

A key-path note's `Q` is the holder's own key, used as is with no BIP-86
tweak. The wallet keeps `sk`, and the mint only ever sees `cp1<Q>`. To spend
the note you hand over `ck1<Q || sig>`, a BIP-340 signature by `sk` over the
key-path sighash for that mint's domain, with an all-zero auxiliary input: one
key has exactly one `ck1` per mint, and seed recovery reproduces it byte for
byte. The mint's certificate, `cs1`, carries the amount in its human-readable
part and the mint's signature over that amount and `hex(Q)`.

```rust
use lnurlcash_core::cash::derive_cash_root;
use lnurlcash_core::recoverable::*;
use lnurlcash_core::check_note;

let node = derive_cash_address_node(&derive_cash_root(&seed)?, "mint.example")?;
let branch = cash_node_to_cx1(&node)?;
let cx1 = encode_cx1(&branch.pubkey_x_only, &branch.chain_code); // watch-only

// purpose: PURPOSE_WALLET (0), PURPOSE_CHANGE (1) or PURPOSE_LIGHTNING_ADDRESS (2)
let pk = derive_note_pubkey(&branch.pubkey_x_only, &branch.chain_code, PURPOSE_WALLET, i)?; // what a watcher derives
let sk = derive_note_secret_key(&node.private_key, &node.chain_code, PURPOSE_WALLET, i)?;
let ck1 = encode_ck1(&sign_note_ownership(&sk, note_url)?); // spends it at this mint only

let cs1 = encode_cs1_with_amount(amount_msat, &mint_signature);
let check = check_note(&ck1, note_url, amount_msat, &cs1, &mint_pubkey); // offline
assert!(check.is_some_and(|check| check.is_verified()));
```

A `cp1` whose key is not a curve point is refused everywhere: no spend could
ever open it, so a note minted to one is value destroyed. To spend a note
whose `Q` commits to a script tree by its key path, sign with
`spend::taproot_tweak_secret_key`.

The fixed-HRP `encode_cs1`/`decode_cs1` functions remain available for notes
certified before the amount-bearing form; `decode_any_cs1` accepts either. A
certificate from a mint that predates taproot is over a bearer note's `h`
rather than its `Q`; `check_note` and `check_note_certificate` still read it,
and say so (`CertifiedOver::LegacyHash`).

A note's tweak is `tagged_hash("LNURLcash/derive", P || chaincode ||
ser32(purpose) || ser32(i))`. `purpose` keeps three counters apart on one
branch: `PURPOSE_WALLET` (0) for the wallet's own notes and a split's `p1`,
`PURPOSE_CHANGE` (1) for a split's change `p2`, and
`PURPOSE_LIGHTNING_ADDRESS` (2) for notes a SERVICE credits by Lightning
Address auto-mint or internal transfer. One `cx1` covers all three; scan each
with its own gap limit on restore.

Certificates travel as `c` (and `c2` for a split's change) in withdraw
responses and the informational GET, and a certified note URL carries
`&c=<cs1>`. Reading still accepts the legacy `sig`, `sig2` and `&sig=`; only
`c`, `c2` and `&c=` are written. The registration proof's request parameter
stays `sig`.

Registering or unregistering a Lightning Address uses the branch's purpose-0
index-0 private key. `sign_address_proof(&sk0, action, domain, username)` returns the
raw 64-byte BIP-340 proof over
`sha256("LNURLcash:<action>:<domain>:<username>")` (`address_proof_message`
builds the string, `address_proof_digest` the 32 bytes that are signed);
action is `register` or `unregister`, `domain` is the SERVICE's hostname, so a
proof one SERVICE saw cannot be replayed at another, and the username must be
normalised exactly as it is sent.

The wire takes every kind. Any spend goes anywhere a k1 does. An output goes
as `p1`/`p2`, a `cp1` or a bearer note's hash alike, and the
`*_request_with_hash` builders refuse, before anything is sent, one that names
no note. `mint_invoice_request_with_hash` sends it as the comment, and
`build_note_info_url_by_hash` as `?p=`. `note_lookup_of(k1)` is what to look a
note up by without disclosing it. The decoders also read the three older
`ck1` shapes, so existing notes stay spendable: Schnorr over the fixed message
`LNURLcash` (its sha256, or the raw bytes) and the 65-byte recoverable ECDSA
`ck1` before that. Rotate those notes into a current `ck1` rather than issuing
new legacy values.

Three things worth knowing:

- **The branch is the domain node itself**, the draft's literal
  `m/139'/d1/d2/d3/d4`, with the hashing key at `m/139'/0`. Earlier versions
  inserted a `1'` hop (`m/139'/1'/...`); keys derived under that hop are not
  on this branch.
- **A `cx1` links every note on its branch.** It spends nothing, but whoever
  holds it can list every key on the branch and ask the mint about each one.
- **`i` is any u32**, serialised as 4 bytes big-endian, never hardened. A tweak
  at or above the curve order is an error rather than reduced: use the next
  index.

The branch is derived from the host exactly as stored, port included, while a
spend is bound to the bare hostname: `localhost:3338` derives one branch and
signs for `localhost`.

`derive_nostr_address_node(secret_key, host)` roots a branch in a Nostr
identity key for a holder with no BIP-39 words:
`HMAC-SHA256(key = secret key, msg = "LNURLcash/nostr-seed")`, then the same
path. That one is an extension, not LUD-25; a mint sees an ordinary `cx1`
either way.

## Amounts

Integers in milli-satoshis, everywhere, with no exceptions.

## Conformance

Tested against
[lnurlcash-conformance](https://github.com/lnurlcash/lnurlcash-conformance):
language-neutral vectors plus a mock mint that can be told to drop a connection
mid-mutation, sign in the wrong byte order, lie about a note's value, or never
settle a melt.

```bash
cargo test --features client       # needs node and the conformance repo alongside
```

## Reference implementations

Both by dni, both MIT:
[lnurl-mint](https://github.com/dni/lnurl-mint) (the service) and
[lnurl-wallet](https://github.com/dni/lnurl-wallet) (the wallet).

The wider ecosystem — wallets, mints, hardware and the sibling ports — is
indexed in [awesome-lnurlcash](https://github.com/lnurlcash/awesome-lnurlcash).

## License

MIT.
