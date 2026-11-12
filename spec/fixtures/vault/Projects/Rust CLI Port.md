---
tags: [project, rust]
status: active
---

# Rust CLI Port

Porting the photo-renaming shell script to a small Rust binary.

- Parse flags with clap and read the EXIF date with a maintained crate.
- Serialise the rename plan with serde so a dry run can be printed as JSON.
- Keep the old script until cargo test covers every rename rule.

Background reading: [[Ownership and Borrowing]].
