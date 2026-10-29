# Run with Windows PowerShell 5.1, the same host used by enrollment.
$ErrorActionPreference = 'Stop'
$enrollScript = Join-Path $PSScriptRoot 'enroll-windows-machine.ps1'
$global:kanbanTestSshExecutable = (Get-Command ssh.exe).Source
$originalProfile = $env:USERPROFILE
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('kanban-enroll-test-' + [guid]::NewGuid().ToString('N'))
$failures = @()

function Assert-True($condition, [string]$message) {
    if (-not $condition) { throw $message }
}
function Test-SelectedKey([string]$target) {
    $resolved = & $global:kanbanTestSshExecutable -T -G -F $global:kanbanTestconfigPath -- $target
    if ($LASTEXITCODE -ne 0) { throw 'OpenSSH config parsing failed' }
    return [bool]($resolved -contains ('identityfile ' + $global:kanbanTestkeyPath.Replace('\', '/')))
}
# Exercise the real enrollment script; only network/key-generation/Herdr operations
# are replaced. Config selection uses the actual Windows OpenSSH parser.
function ssh {
    if ($args -contains '-EncodedCommand') {
        Assert-True (-not $global:kanbanTestReuse) 'Existing authentication must not write remote authorized keys'
        $global:LASTEXITCODE = 0
    } elseif ($args -contains '-i') {
        $global:LASTEXITCODE = 0
        'TEST-REMOTE'
    } else {
        Assert-True ($args -contains 'BatchMode=yes') 'Implicit authentication must never prompt for a password'
        $global:kanbanTestimplicitChecks++
        $selected = Test-SelectedKey $global:kanbanTesttarget
        $global:LASTEXITCODE = if ($selected -and -not $global:kanbanTestrejectImplicit) { 0 } else { 255 }
    }
}
function ssh-keygen { throw 'Unexpected key generation in enrollment regression' }
function Invoke-TestHerdr {
    $global:kanbanTestherdrCalls++
    Assert-True (Test-SelectedKey $global:kanbanTesttarget) 'Herdr would reconnect without the dedicated key'
    $global:LASTEXITCODE = if ($global:kanbanTestHerdrFailure) { 1 } else { 0 }
}

try {
    New-Item -ItemType Directory -Path $testRoot | Out-Null
    foreach ($case in @('user-ip', 'user-dns', 'alias', 'user-alias', 'legacy-retry', 'legacy-lf', 'already-corrected', 'edited-entry', 'implicit-failure', 'existing', 'existing-default', 'existing-failure', 'existing-herdr-failure')) {
        try {
            $caseRoot = Join-Path $testRoot $case
            $env:USERPROFILE = $caseRoot
            $sshDirectory = Join-Path $caseRoot '.ssh'
            New-Item -ItemType Directory -Path $sshDirectory -Force | Out-Null
            $global:kanbanTesttarget = switch ($case) {
                'alias' { 'devpc' }
                'user-alias' { 'Administrator@devpc' }
                'user-dns' { 'Administrator@remote.example.test' }
                default { 'Administrator@100.69.126.63' }
            }
            $global:kanbanTestReuse = $case.StartsWith('existing')
            $keyName = if ($global:kanbanTestReuse) { 'existing-custom-key' } else { 'herdr_kanban_testkey' }
            $global:kanbanTestkeyPath = Join-Path $sshDirectory $keyName
            [IO.File]::WriteAllText($global:kanbanTestkeyPath, 'test-only placeholder; never used for authentication')
            [IO.File]::WriteAllText($global:kanbanTestkeyPath + '.pub', 'ssh-ed25519 AAAA herdr-kanban')
            $global:kanbanTestconfigPath = Join-Path $sshDirectory 'config'
            $prefix = "# User configuration must survive unchanged`r`nHost devpc`r`n    HostName 100.69.126.63`r`n    User Administrator`r`nHost unrelated`r`n    User someone`r`n"
            $existing = $prefix
            if ($global:kanbanTestReuse) {
                $existing += "`r`nHost 100.69.126.63`r`n    IdentityFile `"$($global:kanbanTestkeyPath.Replace('\', '/'))`"`r`n"
            }
            if ($case -in @('legacy-retry', 'legacy-lf', 'already-corrected', 'edited-entry')) {
                $oldHost = if ($case -eq 'already-corrected') { '100.69.126.63' } else { $global:kanbanTesttarget }
                $existing += "`r`n# Herdr Kanban testkey`r`nHost $oldHost`r`n    IdentityFile `"$($global:kanbanTestkeyPath.Replace('\', '/'))`"`r`n"
                if ($case -eq 'edited-entry') { $existing = $existing.Replace('IdentityFile ', 'IdentityFile C:/user-edited-key # ') }
                if ($case -eq 'legacy-lf') { $existing = $existing.Replace("`r`n", "`n"); $prefix = $prefix.Replace("`r`n", "`n") }
            }
            [IO.File]::WriteAllText($global:kanbanTestconfigPath, $existing)
            $settingsPath = Join-Path $caseRoot 'settings.json'
            $resultPath = Join-Path $caseRoot 'result.json'
            $settings = @{ target=$global:kanbanTesttarget; session='default'; key_id='testkey'; herdr='Invoke-TestHerdr'; operation_id=$case; result=$resultPath }
            if ($case -ne 'existing-default') { $settings.configure_key = -not $global:kanbanTestReuse }
            $settings | ConvertTo-Json | Set-Content -LiteralPath $settingsPath -Encoding UTF8
            $global:kanbanTestherdrCalls = 0
            $global:kanbanTestimplicitChecks = 0
            $global:kanbanTestrejectImplicit = $case -in @('implicit-failure', 'existing-failure')
            $global:kanbanTestHerdrFailure = $case -eq 'existing-herdr-failure'
            & $enrollScript -Settings $settingsPath
            $result = Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json
            if ($global:kanbanTestReuse) {
                $expectedError = switch ($case) { 'existing-failure' { 'machine.ssh_connection_failed' }; 'existing-herdr-failure' { 'machine.profile_setup_failed' }; default { '' } }
                $expectedState = if ($expectedError) { 'failed' } else { 'succeeded' }
                Assert-True ($result.state -eq $expectedState -and $result.error -eq $expectedError) "Existing authentication result was $($result.state): $($result.error)"
                Assert-True ($global:kanbanTestimplicitChecks -eq 1) 'Expected one existing authentication check'
                $expectedHerdrCalls = if ($case -eq 'existing-failure') { 0 } else { 1 }
                Assert-True ($global:kanbanTestherdrCalls -eq $expectedHerdrCalls) 'Unexpected Herdr registration after authentication check'
                Assert-True ([IO.File]::ReadAllText($global:kanbanTestconfigPath) -ceq $existing) 'Existing SSH configuration changed'
                Assert-True (-not (Test-Path (Join-Path $sshDirectory 'herdr_kanban_testkey'))) 'Existing authentication created a dedicated key'
            } elseif ($case -eq 'edited-entry') {
                Assert-True ($result.state -eq 'failed' -and $result.error -eq 'machine.ssh_config_failed') 'Edited entry must require manual inspection'
                Assert-True ([IO.File]::ReadAllText($global:kanbanTestconfigPath) -ceq $existing) 'Edited entry was overwritten'
                Assert-True ($global:kanbanTestherdrCalls -eq 0) 'Herdr must not start after config failure'
            } elseif ($global:kanbanTestrejectImplicit) {
                Assert-True ($result.state -eq 'failed' -and $result.error -eq 'machine.key_verification_failed') 'Implicit authentication failure must stop enrollment'
                Assert-True ($global:kanbanTestherdrCalls -eq 0) 'Herdr must not start after failed implicit authentication'
            } else {
                Assert-True ($result.state -eq 'succeeded') "Enrollment failed: $($result.error)"
                Assert-True ($global:kanbanTestherdrCalls -eq 1) 'Expected one Herdr registration'
                Assert-True ($global:kanbanTestimplicitChecks -eq 1) 'Expected implicit authentication check before Herdr'
                $updated = [IO.File]::ReadAllText($global:kanbanTestconfigPath)
                Assert-True ($updated.StartsWith($prefix)) 'Unrelated user configuration changed'
                Assert-True (-not (Test-SelectedKey 'unrelated')) 'Dedicated key leaked to unrelated host'
                if ($global:kanbanTesttarget.Contains('@')) {
                    $hostPart = $global:kanbanTesttarget.Split('@')[-1]
                    Assert-True (-not (Test-SelectedKey ('anotheruser@' + $hostPart))) 'Dedicated key leaked to another user'
                }
                & $enrollScript -Settings $settingsPath
                $result = Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json
                Assert-True ($result.state -eq 'succeeded') 'Repeated enrollment failed'
                Assert-True ([IO.File]::ReadAllText($global:kanbanTestconfigPath) -ceq $updated) 'Repeated enrollment changed SSH config'
            }
            Write-Host "PASS $case"
        } catch {
            $failures += "$case : $($_.Exception.Message)"
            Write-Host "FAIL $case : $($_.Exception.Message)"
        }
    }
} finally {
    $env:USERPROFILE = $originalProfile
    # Delete only the unique temporary directory created by this test.
    $resolvedRoot = [IO.Path]::GetFullPath($testRoot)
    $tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if ($resolvedRoot.StartsWith($tempBase, [StringComparison]::OrdinalIgnoreCase) -and (Split-Path $resolvedRoot -Leaf) -like 'kanban-enroll-test-*') {
        Remove-Item -LiteralPath $resolvedRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
if ($failures.Count) { throw ($failures -join "`n") }
Write-Host 'All 13 enrollment regression cases passed.'
