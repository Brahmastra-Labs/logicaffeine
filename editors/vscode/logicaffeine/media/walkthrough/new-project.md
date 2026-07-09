## Create a project

```sh
largo new hello
code hello
```

You get a `Largo.toml` manifest and `src/main.lg`:

```logos
## Main
Let greeting be "Hello, World!".
Show greeting.
```

LOGOS source is literate Markdown — `##` headers introduce definitions, and
sentences end with periods. It reads like English because it *is* English.
