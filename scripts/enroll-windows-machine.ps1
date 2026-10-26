param([Parameter(Mandatory=$true)][string]$Settings)
$ErrorActionPreference='Stop'
$resultState='failed'
$resultError='machine.enrollment_failed'
try {
 $config=Get-Content -LiteralPath $Settings -Raw | ConvertFrom-Json
 if ($config.target -notmatch '^[A-Za-z0-9][A-Za-z0-9_.@:-]*$') { throw 'Invalid SSH alias' }
 if ($config.session -notmatch '^[A-Za-z0-9_.-]+$') { throw 'Invalid Herdr session' }
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
 Write-Host 'Enter your SSH password in this terminal. Kanban does not receive or store it.' -ForegroundColor Cyan
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
 if(-not $previous.Contains($marker)){
  $keyPath=$keyFile.Replace('\','/')
  [IO.File]::AppendAllText($configFile,"`r`n$marker`r`nHost $($config.target)`r`n    IdentityFile `"$keyPath`"`r`n",[Text.UTF8Encoding]::new($false))
 }
 $resultError='machine.profile_setup_failed'
 & $config.herdr machine add $config.target --remote-session $config.session --label $config.target
 if($LASTEXITCODE -ne 0){throw 'Herdr machine setup did not complete'}
 $resultState='succeeded'
 $resultError=''
 Write-Host 'Authorization complete. Return to Kanban for connection verification.' -ForegroundColor Green
}catch{Write-Host $_.Exception.Message -ForegroundColor Red}
finally {
 if($config -and $config.result -and $config.operation_id) {
  $result=@{id=$config.operation_id;state=$resultState;error=$resultError} | ConvertTo-Json -Compress
  [IO.File]::WriteAllText($config.result,$result,[Text.UTF8Encoding]::new($false))
 }
}
