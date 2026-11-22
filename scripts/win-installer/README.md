# Installing

Run the installer that matches your computer's processor (`-x64` for Intel and AMD, `-arm64` for ARM) and follow the pages. It speaks the
language of Windows (English or French). Windows asks for administrator rights: the programs go in `Program Files`.

| Option | By default | What it does |
|---|---|---|
| Start at logon | yes | The program runs in the background when you open your session, with its icon next to the clock. This is the setting "Start when I open my session" of the program, you can change it there. |
| Install the driver | yes | Installs the driver that makes the virtual microphones and speakers possible (usbip-win2, made by someone else). It is skipped when it is already on the computer. Your USB devices disconnect for a moment while it is installed. Without it the program still works with your real sound devices. |
| Install the command line | yes | Installs the command line program. |
| Add it to the PATH | yes | Lets you type the command line's name in any terminal. Only offered with the command line. |
| Shortcut on the desktop | no | |

## Updating, repairing, removing

Run the installer again, or the one of a newer version: it sees that the program is installed and offers to **update** (or **repair**, when
it is the same version) or to **remove**. Your settings, graphs and sounds are kept by an update, and so are the options you chose the first time.
You can also remove the program from *Apps → Installed apps* of Windows.

Removing the program asks whether to **delete your data too**: settings, graphs, soundboards, sounds, logs, downloaded modules, the window's
cache, the permanent virtual devices and the devices Windows remembers. The box is empty by default, so that a later installation finds
everything again. It applies to the user who removes the program. The driver is never removed: other programs may use it.

## Without any window

From a terminal with administrator rights:

```
omu-<version>-setup-x64.exe /S
omu-<version>-setup-x64.exe /S /INSTALL_DRIVER=0 /INSTALL_AUTOSTART=0 /INSTALL_CLI=1 /INSTALL_PATH=1 /INSTALL_DESKTOP=1 /D=C:\Tools\Omu
```

* `/INSTALL_AUTOSTART`, `/INSTALL_DRIVER`, `/INSTALL_CLI`, `/INSTALL_PATH`, `/INSTALL_DESKTOP` are 1 (yes) or 0 (no); `/D=` is the folder
  and must come last.
* When the program is already installed, the same command updates it (or repairs it, with the same version) and leaves the options of the
  first installation alone.
* To remove it: `"C:\Program Files\Omu\uninstall.exe" /S`; add `/DELETE_DATA=1` to delete the data too.
