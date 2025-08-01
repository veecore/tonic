# tonic-veecore

> ⚠️ Fork of [tonic](https://github.com/hyperium/tonic) with internal gRPC client optimizations. Use at your own risk. Maintained for use in `veecore` projects.

---

## What is this?

This is a modified version of the [`tonic`](https://crates.io/crates/tonic) crate with internal changes aimed at:

- Reducing gRPC client size
- Improving internal ergonomics
- Removing unnecessary dependencies (in progress)

Published as a separate crate to avoid conflicts and confusion. This fork is not guaranteed to follow upstream releases.

---

## Should you use it?

Use it **only if**:

- You’re aware of the changes and accept the risks
- You're cool with no upstream support
- You're not going to cry if your CI breaks next week

---

## License

MIT — same as upstream. You're free to fork, improve, or ignore.

