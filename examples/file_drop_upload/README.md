# File drop — detecting a drop and reading the file

Drag a file onto the zone; its name, size and type appear and a running count
increments.

```bash
largo test           # runs the ## Test blocks against the tree-walker and the VM
largo serve          # http://localhost:8080
largo build --static # dist/ - upload it anywhere
```

## What it shows

**`When a file is dropped on "#zone" (file: File):`** binds the whole dropped
record. The handler reads `file's name`, `file's size` (its content's length, in
bytes — a browser's own `File.size`) and `file's type` (its MIME type), and the
view shows them only once something has been dropped, the size through
`ui.lg`'s `plural` filter (`{lastSize | plural "byte" "bytes"}`).

**Driving a drop in a test.** The drive sentence from `docs/frontend.md`'s Triggers
section declares every field of the dropped file:

`Raise "filedropped" on "#zone" with a new File with name "photo.png" and type "image/png" and bytes [137, 80, 78, 71].`

Each test therefore asserts the exact name, size and type the handler received, on
the tree-walker, the VM and, under `largo test --browser`, a real Chrome.

## Limits — why this app is "file drop," not "file drop + upload"

**No documented sentence uploads a dropped file's content.** `Upload body to
url as T, then action.` sends `body` as the text of one
`multipart/form-data` part named `file` (`multipartBody (content: Text)` in
`crates/logicaffeine_compile/assets/std/web.lg`; pinned by
`upload_lock::the_part_carries_a_name_and_the_content`). A dropped `File`
carries its content in `bytes`, a `Seq of Int`, and the guides document no
sentence that turns those bytes into the `Text` an `Upload` sends. So nothing
documented turns a drop into an upload of the file itself: `Upload file's name
to …` sends the name, not the file. `upload_lock` pins the request an `Upload` sends, not its `then`
action. This app stops at the half that is complete: detecting the drop and
reading the file's name, size and type.
