Set shell = CreateObject("Shell.Application")
shell.ShellExecute "python.exe", "C:\projects\riff\inspect_header.py", "C:\projects\riff", "runas", 1
WScript.Sleep 8000
