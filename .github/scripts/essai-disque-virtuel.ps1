# Écrit, vérifie et formate un VHD attaché ; le disque système doit rester refusé.
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
cargo build -p bonk --locked
if ($LASTEXITCODE -ne 0) { throw 'compilation' }
$bonk = Join-Path $PWD 'target\debug\bonk.exe'
$vhd = Join-Path $env:RUNNER_TEMP 'essai.vhd'
$image = Join-Path $env:RUNNER_TEMP 'essai.img'
$script = Join-Path $env:RUNNER_TEMP 'diskpart.txt'
"create vdisk file=`"$vhd`" maximum=64 type=fixed`nselect vdisk file=`"$vhd`"`nattach vdisk" | Set-Content $script -Encoding ascii
diskpart /s $script
if ($LASTEXITCODE -ne 0) { throw 'diskpart' }
try {
    $bytes = New-Object byte[] (4MB)
    (New-Object Random 42).NextBytes($bytes)
    $bytes[510] = 0x55; $bytes[511] = 0xAA
    [IO.File]::WriteAllBytes($image, $bytes)

    $liste = & $bonk list --all --allow-virtual --json | ConvertFrom-Json
    $virtuel = @($liste.disks | Where-Object { $_.bus -eq 'virtual' -and $_.eligibility.verdict -eq 'offered' })
    if ($virtuel.Count -ne 1) { throw "disque virtuel introuvable : $($liste | ConvertTo-Json -Depth 8)" }
    $systeme = @($liste.disks | Where-Object { $_.partitions.is_system_volume -contains $true })
    if ($systeme.Count -ne 1 -or $systeme[0].eligibility.verdict -ne 'excluded') { throw 'disque système non interdit' }

    & $bonk write $virtuel[0].id $image --allow-virtual --mode raw --yes
    if ($LASTEXITCODE -ne 0) { throw "write : $LASTEXITCODE" }
    # L'identifiant survit à l'écriture, qui a pourtant remplacé la table.
    & $bonk verify $virtuel[0].id $image --allow-virtual
    if ($LASTEXITCODE -ne 0) { throw "verify : $LASTEXITCODE" }

    # Même parcours par le helper élevé : poignée reçue, verrous tenus par lui.
    $env:BONK_WINDOWS_HELPER = 'always'
    & $bonk write $virtuel[0].id $image --allow-virtual --mode raw --yes
    if ($LASTEXITCODE -ne 0) { throw "write par le helper : $LASTEXITCODE" }
    & $bonk verify $virtuel[0].id $image --allow-virtual
    if ($LASTEXITCODE -ne 0) { throw "verify par le helper : $LASTEXITCODE" }
    & $bonk write $systeme[0].id $image --allow-virtual --mode raw --yes
    if ($LASTEXITCODE -ne 3) { throw "disque système : code $LASTEXITCODE au lieu de 3" }
    # Formatage par le helper, identifiant inchangé : FAT32 par Bonk, exFAT et NTFS par le système.
    foreach ($fs in 'fat32', 'exfat', 'ntfs') {
        & $bonk format $virtuel[0].id --fs $fs --label ESSAI --allow-virtual --yes
        if ($LASTEXITCODE -ne 0) { throw "format $fs : $LASTEXITCODE" }
    }
    Remove-Item Env:BONK_WINDOWS_HELPER
}
finally {
    "select vdisk file=`"$vhd`"`ndetach vdisk" | Set-Content $script -Encoding ascii
    diskpart /s $script
}
exit 0
