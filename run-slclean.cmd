@echo off
REM Mở SLClean. Luôn build trước (lần sau chỉ mất vài giây khi không có gì đổi) để bản đang
REM chạy không bị cũ hơn mã nguồn. Nếu build lỗi mà đã có bản build sẵn thì vẫn mở bản đó.
cd /d "%~dp0"
set "EXE=src-tauri\target\debug\slclean.exe"

where cargo >nul 2>&1
if errorlevel 1 goto :launch

echo Dang kiem tra ban build...
cargo build --manifest-path src-tauri\Cargo.toml
if errorlevel 1 (
  echo.
  echo Build khong thanh cong. Neu SLClean dang mo thi dong lai roi chay lai file nay.
  if not exist "%EXE%" (
    pause
    exit /b 1
  )
  echo Van mo ban build cu...
)

:launch
if not exist "%EXE%" (
  echo Khong tim thay %EXE% va khong build duoc. Can Rust toolchain de build lan dau.
  pause
  exit /b 1
)
start "" "%EXE%"
