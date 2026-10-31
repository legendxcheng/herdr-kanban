Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$fixture = Join-Path ([IO.Path]::GetTempPath()) ('kanban-key-test-' + [guid]::NewGuid().ToString('N'))
$secrets = Join-Path $fixture '.secrets'
New-Item -ItemType Directory -Path $secrets -Force | Out-Null
$keyPath = Join-Path $secrets 'deepseek-api-key.txt'
$validator = Join-Path $PSScriptRoot 'check-deepseek-key.ps1'
$cases = @(
    @{ Name='missing'; Value=$null; Valid=$false },
    @{ Name='empty'; Value=''; Valid=$false },
    @{ Name='whitespace'; Value=" `r`n"; Valid=$false },
    @{ Name='prefixed'; Value='Bearer dummy-key'; Valid=$false },
    @{ Name='multiline'; Value="dummy`nkey"; Valid=$false },
    @{ Name='oversize'; Value=('a' * 8193); Valid=$false },
    @{ Name='valid'; Value='dummy-key'; Valid=$true },
    @{ Name='bom-and-newline'; Value=([char]0xFEFF + "dummy-key`r`n"); Valid=$true }
)
try {
    foreach ($case in $cases) {
        if ($null -ne $case.Value) { [IO.File]::WriteAllText($keyPath, $case.Value) }
        $accepted = $true
        try {
            $output = & $validator -RepoRoot $fixture 2>&1 | Out-String
            if ($output.Trim().Length -gt 0) { throw 'Validator must not emit credential data.' }
        } catch {
            $accepted = $false
            if ($_.ToString().Contains('dummy-key')) { throw 'Validator exposed a test credential.' }
        }
        if ($accepted -ne $case.Valid) { throw ('Unexpected key validation result: ' + $case.Name) }
    }
    Write-Host ('PASS: ' + $cases.Count + ' private-key packaging checks')
} finally {
    # Only remove the two files/directories created above; no recursive deletion.
    if (Test-Path -LiteralPath $keyPath) { Remove-Item -LiteralPath $keyPath }
    Remove-Item -LiteralPath $secrets
    Remove-Item -LiteralPath $fixture
}
