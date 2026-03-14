@echo off
:: Internal helper — launched by test_local.bat in a new window.
:: Activates the venv and starts the mock server.
setlocal
set "HERE=%~dp0"
call "%HERE%..\venv\Scripts\activate.bat"
python "%HERE%mock_server.py" --interval 4
echo.
echo  Mock server stopped.
pause
endlocal
