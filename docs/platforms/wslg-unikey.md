# Native UniKey / WSLg reproduction

## Verified environment and failure

The visible window was Atlas (Ubuntu-24.04), a Linux Tauri/WebKitGTK app displayed
through WSLg. Windows UniKey 4.6 RC2 was running with Unicode, Telex, Vietnamese
mode on and `UseUnicodeClipboard=0`. This is not a native Windows WebView2 window.

Physical-style Windows key events were sent to the actual focused chat field,
without sending the message. Typing `tieengs Vieetj` produced `t V`: replacement
keystrokes removed characters but the composed Unicode characters did not arrive.
A standalone GTK Entry, containing no Atlas code or JavaScript, produced the same
result. Separately, the Linux GTK VIQR input module composed `tie^'ng Vie^.t` into
`tiếng Việt` in the native Tauri app on an isolated X11 display.

This local comparison isolates the observed failure to the Windows UniKey/WSLg
input path. It does not establish that every WSLg version or every input method
has the same failure. Earlier Chromium CDP and DOM tests bypassed that path and
therefore did not reproduce it.

## Applied compatibility fix and acceptance

In UniKey, opened Ctrl+Shift+F5 → Mở rộng and enabled **Luôn sử dụng clipboard cho
Unicode** (`UseUnicodeClipboard=1`). No other encoding, input method or Vietnamese
mode preference changed. The option is global and can replace clipboard data
while typing; it is not a new Atlas setting or automatic clipboard access.

Repeated the Windows-side typing test in the same actual Tauri chat:

- `tieengs Vieetj` → `tiếng Việt`.
- `Xin chaof, tooi ddang gox tieengs Vieetj trong Atlas.` at 25 ms per key →
  `Xin chào, tôi đang gõ tiếng Việt trong Atlas.`

The compatibility option was left enabled as the applied fix. The clipboard
content from before the test was retained privately in memory and restored after
verification. The original empty chat draft was restored; no prompt was sent or
conversation saved by the test. Temporary GTK and isolated X11 apps were closed.
Screenshots and local UI automation stayed outside the repository; user chat
history/screenshots were not added to Git.

The option is also described for applications that only accept composed Unicode
through clipboard in the [official UniKey FAQ](https://www.unikey.org/support/ukfaq.html).

## Native Windows build attempt

The same source compiled with the installed Windows MSVC Rust toolchain using
`cargo.exe build --locked` and a separate Windows temporary target directory.
Windows Application Control then rejected the unsigned executable with
“An Application Control policy has blocked this file”. No policy was disabled or
bypassed. Consequently native Windows/WebView2 acceptance was not claimed.

The verified fix for this session is the UniKey compatibility setting above,
not another frontend rewrite or a JavaScript Telex implementation.
