param(
    [string]$Branch = "main",
    [switch]$NoFetch,
    [switch]$NoSquash
)

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\\..")
Set-Location $repoRoot

$remote = "appfs-repo"
$prefix = "appfs"

if (-not (git rev-parse --is-inside-work-tree 2>$null)) {
    throw "This script must run inside the appfs-platform git repository."
}

if (-not (git remote get-url $remote 2>$null)) {
    throw "Required remote '$remote' is not configured."
}

$status = git status --porcelain
if ($status) {
    throw "Worktree must be clean before syncing $prefix."
}

if (-not $NoFetch) {
    git fetch $remote $Branch
}

$args = @(
    "subtree",
    "pull",
    "--prefix=$prefix",
    $remote,
    $Branch
)

if (-not $NoSquash) {
    $args += "--squash"
}

$args += @(
    "-m",
    "Sync $prefix from $remote/$Branch"
)

git @args
