# Adds or removes a folder from the machine PATH, keeping its REG_EXPAND_SZ type.
param([string]$Dir, [switch]$Remove)
$key = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey('SYSTEM\CurrentControlSet\Control\Session Manager\Environment', $true)
$path = $key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
$parts = @($path -split ';' | Where-Object { $_ -ne '' -and $_.TrimEnd('\') -ne $Dir.TrimEnd('\') })
if (-not $Remove) { $parts += $Dir }
$key.SetValue('Path', ($parts -join ';'), [Microsoft.Win32.RegistryValueKind]::ExpandString)
$key.Close()
