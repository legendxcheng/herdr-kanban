param([Parameter(Mandatory=$true)][string]$Settings)
$ErrorActionPreference='Stop'
$resultState='failed'
$resultError='machine.enrollment_failed'
try {
 $config=Get-Content -LiteralPath $Settings -Raw | ConvertFrom-Json
 if ($config.target -notmatch '^[A-Za-z0-9][A-Za-z0-9_.@:-]*$') { throw 'Invalid SSH alias' }
 $targetParts=$config.target.Split('@')
 if ($targetParts.Count -gt 2 -or $targetParts[-1] -notmatch '^[A-Za-z0-9][A-Za-z0-9_.:-]*$') { throw 'Invalid SSH target' }
 if ($config.session -notmatch '^[A-Za-z0-9_.-]+$') { throw 'Invalid Herdr session' }
 if ($config.configure_key -eq $true) {
 $sshDir=Join-Path $env:USERPROFILE '.ssh'
 $resultError='machine.key_setup_failed'
 $keyFile=Join-Path $sshDir ('herdr_kanban_'+$config.key_id)
 New-Item -ItemType Directory -Force $sshDir | Out-Null
 if (-not (Test-Path -LiteralPath $keyFile)) {
  if ($PSVersionTable.PSVersion.Major -lt 7) { & ssh-keygen -t ed25519 -f $keyFile -N '""' -C 'herdr-kanban' }
  else { & ssh-keygen -t ed25519 -f $keyFile -N '' -C 'herdr-kanban' }
  if ($LASTEXITCODE -ne 0) { throw 'Could not create SSH key' }
 }
 $publicKey=[IO.File]::ReadAllText($keyFile+'.pub').Trim()
 if ($publicKey -notmatch '^ssh-ed25519 [A-Za-z0-9+/=]+ herdr-kanban$') { throw 'Unexpected public key' }
 $remote=@'
$ErrorActionPreference='Stop'
$key='__PUBLIC_KEY__'
$identity=[Security.Principal.WindowsIdentity]::GetCurrent()
$isAdmin=@($identity.Groups | ForEach-Object { $_.Value }) -contains 'S-1-5-32-544'
$file=if($isAdmin){Join-Path $env:ProgramData 'ssh/administrators_authorized_keys'}else{Join-Path $env:USERPROFILE '.ssh/authorized_keys'}
$exists=Test-Path -LiteralPath $file
New-Item -ItemType Directory -Force (Split-Path $file) | Out-Null
$previous=if($exists){[IO.File]::ReadAllText($file)}else{''}
if(@($previous -split '\r?\n') -notcontains $key){[IO.File]::AppendAllText($file,"`r`n$key`r`n",[Text.UTF8Encoding]::new($false))}
if(-not $exists -and $isAdmin){
 & icacls $file /inheritance:r /grant '*S-1-5-32-544:F' /grant '*S-1-5-18:F' | Out-Null
 if($LASTEXITCODE -ne 0){throw 'Could not secure authorized_keys'}
}
'@
 $encoded=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($remote.Replace('__PUBLIC_KEY__',$publicKey)))
 Write-Host 'Configuring a dedicated SSH key. If SSH asks for a password, enter it here; Kanban does not receive or store it.' -ForegroundColor Cyan
 $resultError='machine.authorization_failed'
 & ssh -o ConnectTimeout=30 -- $config.target powershell -NoLogo -NoProfile -EncodedCommand $encoded
 if($LASTEXITCODE -ne 0){throw 'SSH authorization failed or timed out. Retry onboarding.'}
 $resultError='machine.key_verification_failed'
 & ssh -o BatchMode=yes -o ConnectTimeout=15 -i $keyFile -- $config.target hostname
 if($LASTEXITCODE -ne 0){throw 'Dedicated key verification failed'}
 $resultError='machine.ssh_config_failed'
 $configFile=Join-Path $sshDir 'config'
 $previous=if(Test-Path -LiteralPath $configFile){[IO.File]::ReadAllText($configFile)}else{''}
 $marker='# Herdr Kanban '+$config.key_id
 $keyPath=$keyFile.Replace('\','/')
 # OpenSSH matches Host against the host portion, never user@host. Use
 # originalhost to preserve alias matching, and scope explicit users separately.
 $selector=if($targetParts.Count -eq 2){"Match originalhost $($targetParts[1]) user $($targetParts[0])"}else{"Host $($config.target)"}
 $block="$marker ssh-config-v2`r`n$selector`r`n    IdentityFile `"$keyPath`"`r`n"
 if(-not $previous.Contains($block)){
  # Repair only the exact stanza emitted by older enrollment (including the
  # documented manual Host correction). Preserve all surrounding user content.
  $oldHosts=@($config.target,$targetParts[-1]) | Select-Object -Unique
  $hostPattern=($oldHosts | ForEach-Object { [regex]::Escape($_) }) -join '|'
  $legacyPattern='(?m)^'+[regex]::Escape($marker)+'\r?\nHost (?:'+$hostPattern+')\r?\n    IdentityFile "'+[regex]::Escape($keyPath)+'"(?:\r?\n|$)'
  $legacy=[regex]::new($legacyPattern)
  if($legacy.IsMatch($previous)){
   $updated=$legacy.Replace($previous,[System.Text.RegularExpressions.MatchEvaluator]{param($match) $block})
  }elseif($previous -match ('(?m)^'+[regex]::Escape($marker)+'(?: ssh-config-v2)?\r?$')){
   throw 'The existing Kanban SSH entry was edited. Check its Host/Match and IdentityFile before retrying.'
  }else{
   $updated=$previous+"`r`n"+$block
  }
  [IO.File]::WriteAllText($configFile,$updated,[Text.UTF8Encoding]::new($false))
 }
 # Herdr and subsequent background requests do not pass -i. Verify that exact
 # authentication path before registration instead of asking for another password.
 $resultError='machine.key_verification_failed'
 & ssh -o BatchMode=yes -o ConnectTimeout=15 -- $config.target hostname
 if($LASTEXITCODE -ne 0){throw 'SSH configuration verification failed; Herdr registration was not started'}
 } else {
  Write-Host 'Checking your existing SSH configuration. Kanban will not provision a new key.' -ForegroundColor Cyan
  $resultError='machine.ssh_connection_failed'
  & ssh -o BatchMode=yes -o ConnectTimeout=15 -- $config.target hostname
  if($LASTEXITCODE -ne 0){throw 'Existing SSH connection failed. Check the target, network and authentication, or choose Configure SSH key in Kanban.'}
 }
 $resultError='machine.profile_setup_failed'
 & $config.herdr machine add $config.target --remote-session $config.session --label $config.target
 if($LASTEXITCODE -ne 0){throw 'Herdr machine setup did not complete'}
 $resultState='succeeded'
 $resultError=''
 Write-Host 'Remote computer setup complete. Return to Kanban for connection verification.' -ForegroundColor Green
}catch{Write-Host $_.Exception.Message -ForegroundColor Red}
finally {
 if($config -and $config.result -and $config.operation_id) {
  $result=@{id=$config.operation_id;state=$resultState;error=$resultError} | ConvertTo-Json -Compress
  [IO.File]::WriteAllText($config.result,$result,[Text.UTF8Encoding]::new($false))
 }
}
