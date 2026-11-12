---
sheet: books
filename: "{title}"
key: [title]
columns:
  title: {type: string, required: true, max_length: 200}
  author: {type: string, required: true, max_length: 120}
  status: {type: enum, required: true, values: [Queued, Reading, Finished, Abandoned]}
  finished: {type: date}
  rating: {type: number}
  link: {type: url, max_length: 500}
---

One note per book. Rows are never deleted: abandoned books keep status Abandoned.
