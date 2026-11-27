# xmip-core-transport-iec-60870-5-104

IEC 60870-5-104 transport: one ASDU is one Stream, the APCI sequence numbers beside it; a Location listens as the controlled station or connects as the controlling one. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

A Receive Location keeps its listener, bound as its transport is opened (`Configured::listening`; on its first receive until 2026-11-16) (`transport::serving::Serving`): a peer that connects between two receives is queued and taken by the next, where until 2026-09-27 each receive bound a listener of its own and a peer between receives was refused.

## Acknowledgement

The controlling station is acknowledged after the whole receive cycle: it
waits for the S frame that acknowledges its I frame. Accepted sends that S
frame. Refused confirms a command (cause activation or deactivation)
negatively: the ASDU mirrored with the confirmation cause and the P/N bit set
(IEC 60870-5-101 section 7.2.3), in an I frame that acknowledges it, so it is
not sent again; a Send Location's send fails permanent there. Any other ASDU
has no negative confirmation in the standard, so Refused acknowledges it with
its S frame: taken and not sent again, the refusal audited in Xmip. Failed
closes the connection unacknowledged, which is how IEC 104 has a controlling
station send what was not acknowledged again; a Send Location's send fails
retryable there. An ASDU let go without a verdict closes the
connection the same way (`transport::answer::Answer`). Each ASDU arrives
whole, and a Receive Location
keeps the controlling stations' connections open between their I frames
(`transport::serving::Serving`), taking the next ASDU from whichever sends
first; until 2026-10-02 a receive read one station's ASDUs until it said
STOPDT or closed, and acknowledged each as it read it. A Send Location keeps
its connection to each controlled station (`transport::Pool`), STARTDT said
once and I frame after I frame in sequence on it; a negative confirmation
keeps the connection, and a close unacknowledged ends it, the next send
opening another. Until 2026-10-21 every send connected, said STARTDT, sent
its one I frame and STOPDT.

## The declaration

What a Location of this technology is configured with is two `const`s in its `declaration` module, `src/declaration.rs`, beside the code, which `Configured` reads (ADR-0064, amendment 2026-11-26): `declaration::SETTINGS`, `timeout`, and `SCHEMES`, `iec104`. Built with its `declaration` feature alone (`--no-default-features --features declaration`), the crate is those and nothing more: its code is behind the default feature. So the library the surfaces load carries it without the code, through the gathering crate xmip-core-declaration (open problem 48), and the language server and the desktop suggest and check an IEC 104 Location's settings though no node in their process loaded the technology. `test/Declaration.Test.ps1` in the estate holds that the declaration builds nothing the transport capability does not.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
