X5 / D1 device verification (2026-09-29/30) - recovery notes for the owner
==========================================================================

What was set up on this machine (nothing outside these paths was changed; no registry, IME setting or config.toml edits)
- C:\Users\cuzic\awase-dv          : extra git worktree of the awase repo (detached at origin/develop 8fe78f34) with a local, uncommitted
                                     patch to examples/chrome_probe.rs. Own target\debug\awase.exe + awase.log. It has no config.toml (defaults).
                                     Registered in the main repo's .git/worktrees. Remove with:
                                       git -C C:\Users\cuzic\scoop\persist\msys2\home\cuzic\awase worktree remove --force C:\Users\cuzic\awase-dv
- C:\Users\cuzic\dv-out\           : logs of the test runs (probe logs, test awase logs, session.status).
- C:\Users\cuzic\dv-x5.ps1, dv-x5-session.ps1, dv-patch.patch : helper scripts (safe to delete).

The test procedure STOPS your awase and starts a second awase (from awase-dv) with RUST_LOG=debug and AWASE_TEST_INJECTION=1.
It always tries to start your original awase again at the end.

Your original awase was started as:  target/debug/awase.exe  (cwd = C:\Users\cuzic\scoop\persist\msys2\home\cuzic\awase, no arguments)
To bring it back by hand (PowerShell):
  Get-Process awase | Stop-Process -Force        # stop ALL awase (test one and any duplicate)
  Start-Process C:\Users\cuzic\scoop\persist\msys2\home\cuzic\awase\target\debug\awase.exe -WorkingDirectory C:\Users\cuzic\scoop\persist\msys2\home\cuzic\awase
Check: exactly one awase.exe, path = ...\home\cuzic\awase\target\debug\awase.exe (NOT ...\awase-dv\...).

If the background session script does not finish (dv-out\session.status has no 'done' / 'aborted' / 'gave up' line):
  Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" | ? { $_.CommandLine -like '*dv-x5*' } | % { Stop-Process -Id $_.ProcessId -Force }
  Get-Process chrome_probe -ErrorAction SilentlyContinue | Stop-Process -Force
  Get-CimInstance Win32_Process -Filter "Name='chrome.exe'" | ? { $_.CommandLine -like '*chrome_probe_profile*' } | % { Stop-Process -Id $_.ProcessId -Force }
  (then restore awase as above). The display-awake request held by the session script ends with the script process.
