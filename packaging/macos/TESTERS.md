# Trying supersilvia on your Mac

Thank you for testing. This build is early and not signed by Apple yet, so macOS will make you
confirm a few things the first time. None of it changes anything else on your Mac.

**This is an alpha.** Projects you save with it may not open in the next build: the file
format can still change without warning, and nothing converts old files. Keep anything you
care about as a render or a Snap as well.

Questions and news are on the supersilvia Discord, https://discord.gg/3RMcCnHbf. Bugs go in its
bug report channel, https://discord.gg/prRuJ6rvy.

**You need** a Mac with Apple Silicon (M1 or later) on macOS 14.2 Sonoma or later. Intel Macs
are not supported. You do not need Homebrew, GStreamer or anything else: everything the app
uses is inside it.

## Install

1. Download `supersilvia-<version>-macos-arm64.dmg` (or the `.zip`, if that is what you were sent).
2. Open the `.dmg` and drag **supersilvia** onto the **Applications** folder beside it. From a
   `.zip`, double-click it and drag the app it unpacks into Applications.
3. Eject the disk image.

## The first launch

macOS does not recognise the app yet, so the first time goes like this:

1. Open supersilvia from Applications. macOS says it cannot verify the app. Click **Done**
   (not *Move to Trash*).
2. Open **System Settings ▸ Privacy & Security** and scroll down to **Security**. There is a
   line saying supersilvia was blocked. Click **Open Anyway**, and confirm with your password
   or Touch ID.
3. Open supersilvia again and click **Open**.

After that it opens like any other app. On macOS 14 you can also right-click the app, choose
**Open** and confirm, which does the same thing.

## Permissions

supersilvia asks the first time you use each of these, and only then:

- **Camera**, when you pick a camera (an iPhone nearby with Continuity Camera shows up too).
- **Microphone**, when you pick a microphone or audio interface.
- **System audio**, when you pick *System audio* to drive visuals from what your Mac plays.
- **Local network**, when you use NDI.

If you said no by accident, turn it on in **System Settings ▸ Privacy & Security** under that
heading, then quit and reopen supersilvia.

**Screen capture** uses macOS's own picker, where you choose the screen or window. If macOS
asks for **Screen & System Audio Recording** permission instead, allow it in System Settings
and then **quit and reopen supersilvia**: macOS only applies that permission after a relaunch.

**Each new build asks again.** Until the app is signed by Apple, macOS treats every new build as
a different app, so after updating you will be asked for the camera, microphone and the rest
once more, and may need *Open Anyway* again. That is expected.

## NDI

To send or receive NDI video, install NDI's free **NDI 6 Runtime** from
[ndi.link/NDIRedistV6Apple](https://ndi.link/NDIRedistV6Apple), then restart supersilvia, and
allow **Local network** when macOS asks. supersilvia does not include the runtime, because
NDI's licence has you install it yourself.

**NDI Tools alone is not enough**: its apps carry private copies of the runtime and install
nothing supersilvia can use. It is still handy for testing, with Video Monitor to watch what
supersilvia sends and Test Patterns to receive.

NDI® is a registered trademark of Vizrt NDI AB. [https://ndi.video](https://ndi.video)

## Where your things are

| | |
| --- | --- |
| projects | `~/Documents/supersilvia/`, one folder each. **supersilvia ▸ Settings… ▸ Files** shows it, opens it in the Finder, and **Change…** picks another. A project saved somewhere else stays where you saved it |
| inside a project | `assets/` the media you imported, `renders/` your renders, `snaps/` your Snaps, `cache/` the re-encoded copies of your clips (made again if you delete it), `.autosave/` unsaved changes (below) |
| preferences | `~/Library/Application Support/supersilvia/preferences.json`. Settings ▸ Files opens it in your text editor; edit it with supersilvia closed, since it writes its own copy while it runs. A file it cannot read is kept as `preferences.json.bad` |
| the log | `~/Library/Application Support/supersilvia/logs/`: `supersilvia.log` for the run going on, `previous.log` for the one before |
| GStreamer's plugin list | `~/.cache/gstreamer-1.0/`, shared with every GStreamer app |

**macOS asks once whether supersilvia may use your Documents folder**, the first time it
starts — and again after each new build, as with the camera, until the app is signed by
Apple. Allow it: your projects live there. If you clicked **Don't Allow**, supersilvia says so
and keeps working on an empty, unsaved project; turn it on in **System Settings ▸ Privacy &
Security ▸ Files and Folders ▸ supersilvia ▸ Documents Folder** and reopen it.

**If iCloud's Desktop & Documents sync is on**, your projects are in iCloud Drive: everything
in them uploads, renders and the clip cache included, and with **Optimize Mac Storage** a
clip may be moved off your Mac and only downloaded again when a project opens it. To keep
projects out of iCloud, choose a folder outside Documents with **Settings ▸ Files ▸
Change…**.

**Project ▸ Show project folder** opens the folder of the project you are in. After a Snap or
a render, the note at the foot of the window that says where it went has a **Show** that
selects the file in the Finder. The **file** line at the foot of the Status box's Project section
(Settings ▸ Performance ▸ Show the Status box) keeps a **▸ show** that does the same.

## If supersilvia closes unexpectedly

The next time it starts, it says so first — **supersilvia closed unexpectedly** — with the
reason its log gives, if it wrote one, and **Show log**, which shows that log in the Finder.

If it cannot start at all — there is no GPU it can use, or its window does not open — a box
says why and where its log is.

While a project has unsaved changes, supersilvia writes them every 30 seconds into the
project's own `.autosave/` folder, and a real Save deletes it. The next time that project
opens — and supersilvia opens your last project when it starts — it asks whether to recover
the changes, saying how old they are. **Recover** puts them on screen, still unsaved, so save
to keep them; **Discard** throws them away. At most the last 30 seconds of work are lost.

## Licences and source

**supersilvia ▸ About supersilvia** in the menu bar names the licences, and the app carries their texts
in `supersilvia.app/Contents/Resources/licenses/`. The download may come with
`gstreamer-1.28.7-source.tar` beside the `.dmg`: that is the source code of GStreamer, which
the app carries, offered as its licence asks, with a `SOURCE-OFFER.txt` inside saying what each
file is. You do not need
them to run supersilvia.

## Sending a bug report

Choose **Help ▸ Report a problem…** in the menu bar. A small window asks **What's up?** — say
what you did, what you expected and what happened instead, and your Mac's model (Apple menu ▸
About This Mac) — and, if you like, your name or Discord handle. Under that it shows what it adds by itself: the version, your macOS version
and GPU, what supersilvia finds of GStreamer, and the end of the log, and of the last run's
log if that one closed unexpectedly.

Press **Copy report**, then **Open the Discord bug channel**
(https://discord.gg/prRuJ6rvy) and paste it in a new thread, one thread a bug. **Open a
GitHub issue** is the other place to paste it.

If supersilvia cannot start at all, send the files in
`~/Library/Application Support/supersilvia/logs/` instead: in the Finder, **Go ▸ Go to
Folder…** and paste that path.

If supersilvia crashed, macOS shows a report window: click **Report…**, copy all of the text,
and send that too. Past crashes are in **Console** (Applications ▸ Utilities) under **Crash
Reports**, named `supersilvia`.

## Removing it

Drag **supersilvia** from Applications to the Trash. To remove what it saved too, delete
`~/Library/Application Support/supersilvia/` and — only if you do not want your projects,
renders and Snaps — `~/Documents/supersilvia/`, or the projects folder Settings ▸ Files names
if you changed it. `~/.cache/gstreamer-1.0/` is GStreamer's list of its plugins, made again
whenever an app that uses GStreamer starts; delete it too if nothing else of yours does. To
make macOS forget what you allowed it — the camera, the microphone, Documents and the rest —
run `tccutil reset All io.github.figrita.supersilvia` in Terminal.
