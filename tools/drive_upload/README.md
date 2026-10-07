# The upload script: stations' files straight into the project's Google Drive

Any Aether HF station can send its logs and sessions to the project with one button: **Log**
tab → **Send files…** → *Send my files to the Aether project* → **Send**. No email, no
attachment, no code. A recording is uncompressed audio, about 5.8 MB a minute, so a Test
session's zip is past what an email carries anyway. The files land in a folder in the project
owner's Google Drive, and the owner gets an email with a link to each one.

What does it is this small script, running in the owner's Google account. Its address is built
into the panel (`PROJECT_UPLOAD_URL` in `app/ui/app.js`) and is public; nothing secret is in the
Aether program. What keeps a public address harmless is what the script allows: one folder, files
it cannot read back, at most `DAILY_UPLOADS` (20) uploads and `DAILY_BYTES` (2 GB) a day, 400 MB
a file, and an email to the owner for every one.

## Setting it up (once, about ten minutes)

1. Open <https://script.google.com>, signed in to the account whose Drive should receive the
   files, and choose **New project**. Name it *Aether HF uploads*.
2. Replace everything in `Code.gs` with this folder's [`Code.gs`](Code.gs). At the top, check
   `OWNER_CALL` (your callsign) and the daily limits.
3. **Project Settings** (the gear) → tick *Show "appsscript.json" manifest file in editor*. Back
   in the editor, replace `appsscript.json` with this folder's
   [`appsscript.json`](appsscript.json).
4. Pick **setup** in the function list beside *Run*, and press **Run**. Google asks you to
   authorise the script: *Review permissions* → your account → *Advanced* → *Go to Aether HF
   uploads (unsafe)* → *Allow*. Google shows that warning for any script nobody has submitted
   for review; this one is yours. The log shows the new folder, *Aether HF uploads*.
5. **Deploy → New deployment** → the gear beside *Select type* → **Web app**. Set *Execute as*
   to **Me** and *Who has access* to **Anyone**. Press **Deploy** and copy the *Web app URL*.
   It ends in `/exec`, and it goes into `PROJECT_UPLOAD_URL`.
6. Check it from a terminal in the Aether HF folder:

   ```
   python tools/drive_upload/try_upload.py https://script.google.com/macros/s/…/exec
   ```

   It sends a 1 MB test file the way a station does and prints its link. You get an email.
   Delete the test file from the folder if you like.

## Changing the script without changing its address

The address belongs to the *deployment*, not to the code, and every station's panel has it
built in. So after editing `Code.gs`: **Deploy → Manage deployments** → the pencil on the
existing deployment → *Version*: **New version** → **Deploy**. The address stays the same.
(*New deployment* would make a new address, which only a new release of the app could carry.)

## Asking a station for its files

On the panel's **Log** tab: **Send files…** → *Ask a station to send me its files*. Fill in the
station's callsign, how far back, and tick *audio* if you want the recordings. *Write request*
opens an email with a link; you add their address and send it. When they open the link with
Aether HF running, their panel opens the form ready: they press **Send**.

* Sent from the project owner's station (`PROJECT_CALL`), the link asks for the files to the
  project's folder.
* Sent from any other station, the link asks for them by email to the address you give, and
  their panel opens on *Email my files to someone* with it filled in.

A request link from beta.85–87 that names its own upload script and code still works.

## Watching and stopping it

* **today** (pick it, then Run) logs the day's uploads and bytes against the limits.
* To refuse everything for a while, lower `DAILY_UPLOADS` to 0 and deploy a new version as
  above. To stop it for good, *Deploy → Manage deployments → Archive*; the panel's
  *Send* then fails and says to use email.
* Codes (`makeCodes`, `listCodes`, `revokeCode`) are optional now: an upload that carries one
  must name a known code with uploads left and is counted against it. No release of the panel
  sends one any more.

## What it allows

* Up to `DAILY_UPLOADS` uploads and `DAILY_BYTES` a day (UTC), and 400 MB a file (`MAX_BYTES`).
* Each upload goes into the folder under a name with the time, the sending station and the
  zip's name, and its description says who sent it.
* The emails come from your own account to your own account (Apps Script's daily quota is
  100 for a personal account).
* The script runs as you, so it can reach your Drive: anybody with the address can put a file
  into that one folder, within the limits, and nothing more. It cannot read, list or delete
  anything.

How it works: `begin` checks the day's limits, asks Google Drive for a *resumable upload*
session for one file in the folder, using your account's authority, and hands the session's
address to the station. The station sends the zip there in 8 MB pieces, picking up where a
dropped connection left off. Then `finish` checks the file is in the folder and emails you.
The station's side is `core/aetherd/src/upload.rs` (`share.upload` in the control API).
