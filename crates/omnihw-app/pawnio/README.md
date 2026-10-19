# PawnIO installer

`PawnIO_setup.exe` is the unmodified, signed official installer of PawnIO
[2.2.0](https://github.com/namazso/PawnIO.Setup/releases/tag/2.2.0)
(sha256 `1f519a22e47187f70a1379a48ca604981c4fcf694f4e65b734aaa74a9fba3032`), from
<https://pawnio.eu>. Omni-HW embeds it and runs it (silently, elevated) only when the user clicks
"Set up extra sensors" on the Settings page and PawnIO isn't installed yet.

PawnIO is licensed under the GNU GPL 2.0 or later with a special exception for programs that
talk to it only through its device IO control interface, which is all Omni-HW does (`COPYING`
is the GPL text). Driver source: <https://github.com/namazso/PawnIO/tree/2.2.0>; installer
source: <https://github.com/namazso/PawnIO.Setup/tree/2.2.0>.
