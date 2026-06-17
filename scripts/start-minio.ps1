# Start local MinIO for TimeTracker (Windows, no Docker).
# S3 API :9100 — do NOT use :9000 (that port is the Axum API server).
$ErrorActionPreference = 'Stop'
$env:MINIO_ROOT_USER = 'minioadmin'
$env:MINIO_ROOT_PASSWORD = 'minioadmin'
$data = 'C:\minio\data'
$exe = 'C:\minio\minio.exe'
if (-not (Test-Path $exe)) {
  Write-Error "MinIO not found at $exe — see README 'Running storage without Docker'."
}
New-Item -ItemType Directory -Force -Path $data | Out-Null
$existing = Get-NetTCPConnection -LocalPort 9100 -State Listen -ErrorAction SilentlyContinue
if ($existing) {
  Write-Host "MinIO already listening on :9100 (PID $($existing.OwningProcess))."
  exit 0
}
Write-Host "Starting MinIO on :9100 (console http://localhost:9001) ..."
Start-Process -FilePath $exe -ArgumentList 'server', $data, '--address', ':9100', '--console-address', ':9001' -WindowStyle Hidden
