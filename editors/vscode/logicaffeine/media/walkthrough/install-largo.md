## Install the largo build tool

`largo` builds, runs, and proves LOGOS programs. One line:

```sh
curl -fsSL https://logicaffeine.com/install.sh | sh
```

On Windows (PowerShell):

```powershell
irm https://logicaffeine.com/install.ps1 | iex
```

Verify with `largo --version`. The language server is bundled with this
extension — largo is only needed to *run* and *prove* programs.
