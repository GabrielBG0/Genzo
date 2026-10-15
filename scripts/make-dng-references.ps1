# Makes linear, uncompressed reference DNGs with Adobe DNG Converter (Windows) for the
# 0.C corpus. Usage: pwsh scripts/make-dng-references.ps1 [file ...]
# With no arguments it converts every raw file under corpus/raw and corpus/proraw.
# Output: corpus/reference/<file stem>/<file stem>.dng (gitignored).
# Flags: -c no lossy compression, -l linear (demosaiced) output, -u uncompressed.
param([string[]]$Files)
$root = Join-Path (Split-Path $PSScriptRoot) 'corpus'
$exe = 'C:\Program Files\Adobe\Adobe DNG Converter\Adobe DNG Converter.exe'
if (-not $Files) {
    $Files = Get-ChildItem (Join-Path $root 'raw'), (Join-Path $root 'proraw') -Recurse -File |
        Where-Object { $_.Extension -notin '.json', '.md' } | ForEach-Object FullName
}
foreach ($f in $Files) {
    $stem = [IO.Path]::GetFileNameWithoutExtension($f)
    $out = Join-Path $root "reference\$stem"
    New-Item -ItemType Directory -Force $out | Out-Null
    $p = Start-Process $exe -ArgumentList @('-c', '-l', '-u', '-d', "`"$out`"", "`"$f`"") -PassThru -Wait
    "{0}: exit {1}" -f $stem, $p.ExitCode
}
