# tonic-veecore

> ⚠️ Fork of [tonic](https://github.com/hyperium/tonic) with internal gRPC client optimizations. Use at your own risk. Maintained for use in `veecore` projects.

---

## What is this?

This is a modified version of [`tonic`](https://crates.io/crates/tonic), a gRPC implementation originally developed by the fantastic folks at [hyperium](https://github.com/hyperium). Our fork introduces changes with zero concern for general compatibility:

- Smaller gRPC client
- Internally streamlined abstractions
- Dependency purges in progress

We publish this separately to keep our hands free. It’s not meant to track upstream releases.

---

## Should you use it?

Use it **only if**:

- You’re aware of the changes and accept the risks
- You’re aware this isn't upstream-approved
- You're cool with no upstream support
- You're not going to cry if your CI breaks next week

---

## Attribution

Much love and technical admiration to the original [tonic](https://github.com/hyperium/tonic) authors. Without them, this fork wouldn’t exist. We break things only because we can stand on what they've built.

---

## License

MIT — same as upstream.

Fork it. Break it. Make it worse. Or better.
