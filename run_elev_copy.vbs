Set shell = CreateObject("Shell.Application")
shell.ShellExecute "python.exe", "C:\projects\riff\parse_dump.py", "C:\projects\riff", "runas", 1
WScript.Sleep 15000
