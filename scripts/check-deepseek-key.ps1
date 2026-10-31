param([Parameter(Mandatory = $true)][string]$RepoRoot)

$keyPath = Join-Path $RepoRoot '.secrets\deepseek-api-key.txt'
if (-not (Test-Path -LiteralPath $keyPath -PathType Leaf)) {
    throw 'DeepSeek key is missing. Create .secrets/deepseek-api-key.txt before packaging.'
}
try { $key = [IO.File]::ReadAllText($keyPath).TrimStart([char]0xFEFF).Trim() }
catch { throw 'Cannot read the private DeepSeek key file.' }
try {
    if ([string]::IsNullOrEmpty($key) -or $key.Length -gt 8192 -or $key -cmatch '[^\x21-\x7E]') {
        throw 'DeepSeek key must be a nonempty bare ASCII token (no Bearer prefix).'
    }
} finally { $key = $null }
