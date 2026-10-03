# xmip-core-transport-iec-60870-5-104

IEC 60870-5-104 transport: one ASDU is one Stream, the APCI sequence numbers beside it; a Location listens as the controlled station or connects as the controlling one. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

A Receive Location keeps its listener, bound on the first receive (`transport::serving::Serving`): a peer that connects between two receives is queued and taken by the next, where until 2026-09-27 each receive bound a listener of its own and a peer between receives was refused.

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
STOPDT or closed, and acknowledged each as it read it.

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
