---
tags: [rust, concept]
---

# Ownership and Borrowing

Every value in Rust has a single owner; when the owner goes out of scope the value is dropped.

- A shared reference lets many readers look at a value at once.
- A mutable reference is exclusive: one writer and no readers.
- The borrow checker rejects code where a reference could outlive its value.
- Lifetimes are the names the compiler gives to those reference scopes.
