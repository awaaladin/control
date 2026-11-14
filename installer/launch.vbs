' Starts the PULSE engine (rhcsd) hidden, with no console window.
'
' Used by: the Start menu shortcut, "Start PULSE when I sign in", the installer, and the
' pulse: link in the browser (pulse://start). Because a web page can trigger the link, this
' script accepts ONLY a fixed set of arguments and ignores everything else. Nothing from the
' link is ever used as a path, a command or a parameter.
Option Explicit

Dim sh, fso, dir, exe, log, arg, ok, wmi, procs, p, cmd, q

Set sh = CreateObject("WScript.Shell")
Set fso = CreateObject("Scripting.FileSystemObject")
dir = fso.GetParentFolderName(WScript.ScriptFullName)
exe = dir & "\rhcsd.exe"
log = dir & "\rhcsd.log"
q = Chr(34)

' ---- argument check (allow-list) -------------------------------------------------------
If WScript.Arguments.Count > 1 Then WScript.Quit 2
arg = ""
If WScript.Arguments.Count = 1 Then arg = LCase(Trim(WScript.Arguments(0)))
Select Case arg
    Case "", "start", "pulse:start", "pulse:start/", "pulse://start", "pulse://start/"
        ok = True
    Case Else
        ok = False
End Select
If Not ok Then WScript.Quit 2

If Not fso.FileExists(exe) Then WScript.Quit 3

' ---- already running? (this install's rhcsd.exe only) -----------------------------------
Set wmi = GetObject("winmgmts:\\.\root\cimv2")
Set procs = wmi.ExecQuery("SELECT ExecutablePath FROM Win32_Process WHERE Name = 'rhcsd.exe'")
For Each p In procs
    If Not IsNull(p.ExecutablePath) Then
        If LCase(p.ExecutablePath) = LCase(exe) Then WScript.Quit 0
    End If
Next

' ---- keep the log small: past 1 MB the current log becomes rhcsd.log.old ----------------
If fso.FileExists(log) Then
    If fso.GetFile(log).Size > 1048576 Then
        If fso.FileExists(log & ".old") Then fso.DeleteFile log & ".old", True
        fso.MoveFile log, log & ".old"
    End If
End If

' ---- start hidden; the engine finds .env next to it ------------------------------------
sh.CurrentDirectory = dir
cmd = "cmd.exe /c " & q & q & exe & q & " >> " & q & log & q & " 2>&1" & q
sh.Run cmd, 0, False
