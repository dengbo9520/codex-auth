@echo off
setlocal

set "VSVCVARS=C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"

if not exist "%VSVCVARS%" (
  echo vcvars64.bat not found at "%VSVCVARS%"
  exit /b 1
)

call "%VSVCVARS%" >nul
if errorlevel 1 (
  echo Failed to load Visual C++ build environment.
  exit /b 1
)

set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

if /I "%~1"=="--print-env" (
  set
  exit /b 0
)

if "%~1"=="" (
  cmd /k
  exit /b %errorlevel%
)

set "TARGET=%~1"
shift
call "%TARGET%" %*
exit /b %errorlevel%
