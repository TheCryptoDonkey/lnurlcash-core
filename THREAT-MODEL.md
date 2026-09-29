# Threat model

What this library defends against, what it cannot, and what it hands to you.

## What it is

The protocol logic for LNURLcash bearer notes: building requests, classifying
responses, generating replacement note secrets, and verifying mint signatures
offline. It holds no state between calls. An optional HTTP client and an
optional UniFFI surface sit on top.

## Assets

**Spends (`k1`).** Bearer instruments: a bearer note's preimage, a `ck1` or a
`cw1`. Whoever holds one can spend its note with no further authentication.
A preimage works at any mint the note exists at; a `ck1` only at the mint
whose domain it signs for, but anyone who sees it can still spend it there.
Compromise is theft, and it is silent and irreversible: the money is gone
before the previous holder has any way to notice. Note secret keys and branch
nodes are the same asset one step removed.

**Replacement secrets awaiting confirmation.** After a mutation whose outcome
is unknown, the secrets generated in-process may be the only copies of notes
a service has already minted. Losing them destroys the money exactly as
thoroughly as leaking them gives it away.

**Mint pubkeys.** Not secret, but a wrong one makes offline verification
meaningless.

## Trust boundaries

| Party | Trusted for |
| --- | --- |
| the SERVICE (mint) | custody of the sats, and honest accounting. Nothing else. |
| the caller's storage | confidentiality and durability of secrets. This library provides neither. |
| the caller's RNG | unpredictability of replacement secrets. Substitutable, and load-bearing. |
| the network | nothing. |

A mint is trusted with custody by construction — it holds the funds. It is
*not* trusted to describe them accurately, which is why value comes from
`maxWithdrawable` rather than from a note URL's own `amount`, and why a
signed note can be checked against a key the mint published earlier.

## What this library defends against

**A service that keeps a copy of your note.** Rotate, split and merge disclose
only `sha256(secret)` or a `cp1`. The service registers the note under the
taproot output key that names and never sees what spends it. This is the difference between a bearer note and a
receipt, and it is why a service-generated replacement is refused even when
offered (`serverGeneratedSecrets` in the mock mint exercises exactly this).

**A mutation whose outcome is unknown.** Timeouts, dropped connections,
unreadable bodies and unconfirmed 200s are all raised as
`AmbiguousMutationError` carrying the fresh secrets, never as failure.
Requests that provably never left — offline mode, a refused URL, an
unparseable callback — are raised as `RequestRefusedError` instead, which is
safe to treat as "nothing happened".

**A note that answers its own questions.** Every URL fetched, whether scanned
by a user or supplied by a service in its own response, must be https, or
http to loopback or `.onion`. A `data:` URL carrying withdrawRequest JSON
would otherwise mint a self-contained fake note that verifies against
nothing.

**A service that inflates a note.** Every note's certificate, bearer or
key-path, commits to the amount and to the note's output key `Q`. A service
reporting more than it certified fails verification, without the holder
contacting anyone, and so does a `cs1` whose human-readable amount disagrees
with the amount it signs. A certificate from a mint predating taproot, over a
bearer note's hash instead of `Q`, is still read and reported as such. The
tolerant default also admits a bearer output from a no-signer mint; that
particular note has no offline amount proof. Require signatures, or hold a
certified note, where that defence matters.

**A forged spend beside a genuine certificate.** `Q` and its certificate are
both public: a mint hands out `cs1` on a `?p=` lookup to anyone who asks. A
`ck1` states its `Q` in plain sight, so `ck1<Q || junk>` beside a real `cs1`
looks certified. Offline verification (`check_note`, `verify_note_signature`,
`check_note_url`) therefore checks that the spend opens `Q` at the note's
domain as well as the certificate, and needs the domain to do it. A
certificate alone proves issuance, never that the spend in hand is good.

**A spend replayed at another mint.** Every signature signs a sighash whose
prevout is bound to the mint's domain, so a `ck1` one mint has seen fails at
every other. A bearer preimage checks no signature and is bound to no mint,
by design: it is the money wherever the note exists.

**A note minted to a key nothing can open.** A `cp1` whose `Q` is not a curve
point, or an output that is neither a `cp1` nor a bearer hash, is refused
before any request is sent. At a mint that failed to check, naming one would
burn the inputs into a note no spend could ever open.

**A service that swaps your note.** The informational GET checks that the
echoed `k1` names the note queried, by its `Q`, and that a different spend of
it does not fail at the queried domain. Another note means either a
non-compliant service or a note redeemed by somebody else.

**A secret leaking through a query string.** `sig` is stripped before the
informational GET, since the service already knows what it signed.

**A hostile fee advertisement.** Fees of 100% or more are refused at parse
time, and the gross-up search is a binary search rather than a walk — so a
service cannot stall a caller with an extreme fee.

**Integer overflow on realistic amounts.** The proportional fee term is
computed split, because 21M BTC in msat times a high ppm exceeds 64-bit
unsigned. Ports that multiply naively pass every small test and mangle large
ones; the conformance vectors include a case that catches it.

## What it does not defend against

**Storage compromise.** This library never persists anything. If your
storage is readable — an unencrypted database, a synced folder, a debugger,
a crash dump — every note in it is spendable by whoever reads it. Encrypt at
rest, and treat backups as the same exposure.

**A weak RNG.** `ClientConfig::secret_source` is replaceable, which means it can be
replaced badly. A predictable secret is a note anyone can mint themselves. Use the
platform CSPRNG, or a hardware RNG, and nothing else.

**Secrets in logs.** A note URL carries its secret in a query string. A
request logger, an error reporter, a crash handler or an analytics SDK that
records URLs records bearer money. This library never logs; what wraps it
might.

**A malicious or compromised mint.** It holds the funds. It can refuse to
honour a note, vanish, or inflate its liabilities. Offline verification
proves what it *said*, which is useful for exposing it afterwards, and is
not custody.

**The mint-time preimage race.** A freshly minted note's secret is the
invoice preimage, so the service has necessarily seen it, and anyone who saw
the unpaid invoice can poll LUD-21 `verify` for it the moment it settles.
Rotating immediately wins that race; a slow manual flow does not. Do not
publish unpaid mint invoices.

**Traffic analysis.** Every operation reaches the mint directly. The mint
learns your IP, your timing, and which notes move together. Notes are bearer
instruments, not private ones — merging several notes tells the mint they
had one holder. Route over Tor if that matters.

**A timelock is the mint's clock.** A script-path note's time claim is
judged by the mint against its own clock. `check_time_claim` reports what a
mint will say, but a timelock a mint honours is a custodial policy, not a
consensus guarantee, and nothing here presents it as trustless.

**Scripts this library does not run.** Offline, only the bearer hashlock is
evaluated. Any other leaf is reported `Unevaluated`, never guessed at, and
`verify_note_signature` treats it as unverified.

**Anything about the sats themselves.** No custody, no channel management,
no payment routing.

## Deliberate design choices

**Both signature recovery-id orderings are accepted.** The wire format is
`r || s || recovery_id`; lnurl-mint once emitted the reverse. Trying both is
not a weakening: recovering under the wrong ordering yields an unrelated
pubkey, which cannot match the expected one.

**Deprecated schemes are read, never produced.** Three older `ck1` shapes
(Schnorr over the fixed message `LNURLcash` or its sha256, and 65-byte
recoverable ECDSA) and certificates over a bearer note's hash still verify,
because notes held under them are money. Each is reported as legacy so a
holder can rotate; nothing here signs one.

**Errors are typed by whether the request could have been processed**, not by
transport detail. That distinction is the whole safety model, and message
text is not a stable interface — never branch on it.

**No global state.** Offline mode, timeout and RNG live in `ClientConfig`. A
caller that wants certainty nothing reaches the network sets `offline` and gets
a refusal, rather than trusting that no code path happens to make a request.

**The core has no I/O at all.** `protocol` builds requests and parses
responses; nothing in it can reach the network even by accident. The HTTP
client is an optional feature, and the FFI surface excludes it entirely.

## Reporting

See [SECURITY.md](SECURITY.md).
