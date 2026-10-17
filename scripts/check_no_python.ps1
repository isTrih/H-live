# Check the current Git index, including files hidden by ignore rules.
param([string]$Repository = (Split-Path -Parent $PSScriptRoot))
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath $Repository).Path
$gitOptions = @('-c', ('safe.directory=' + $root.Replace('\','/')), '-C', $root)
$paths = & git @gitOptions -c core.quotepath=false ls-files --cached -z
if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect tracked source files' }
$paths = @(($paths -join "`n").Split([char]0, [StringSplitOptions]::RemoveEmptyEntries))
if (-not $paths.Count) { throw 'No tracked files were inspected' }
$forbidden = @($paths | Where-Object {
    $_ -match '(?i)\.(py|pyw|pyi|pyx|pxd|pxi|pyc|pyo|pyd|ipynb)(\.|/|$)' -or
    [IO.Path]::GetFileName($_) -match '(?i)^(pyproject\.toml|Pipfile(\.lock)?|poetry\.lock|requirements[^/]*\.txt|\.python-version|\.pythonrc)$'
})
if ($forbidden.Count) { throw ('Python source/runtime files are prohibited: ' + ($forbidden -join ', ')) }
# Also reject extensionless Python scripts and Python code embedded in docs.
$matches = & git @gitOptions grep --cached -n -I -E '^#!.*(python|pypy)([0-9.]*)?([[:space:]]|$)|^[[:space:]]*(```|~~~)[[:space:]]*(python|py|python[23])([[:space:]]|$)' -- .
$grepExit = $LASTEXITCODE
if ($grepExit -gt 1) { throw 'Cannot inspect source content' }
if ($grepExit -eq 0) { throw ('Python code found in tracked content: ' + ($matches -join ', ')) }
Write-Output "No Python source, bytecode, extension modules, notebooks, manifests or Python code fences in $($paths.Count) tracked files."
# git grep uses 1 to mean no matches; expose a successful check to callers.
$global:LASTEXITCODE = 0
