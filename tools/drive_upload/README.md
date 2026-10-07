# The upload script: other stations' files straight into your Google Drive

When you ask a station for its files, its panel can send them to you with one button, with no
email attachment to forget. A recording is uncompressed audio, about 5.8 MB a minute, so a
Test session's zip is past what an email carries anyway. The files land in a folder in your
Google Drive, and you get an email with a link to each one.

What does it is this small script, running in **your** Google account. It never holds the
other station's password or yours, and nothing secret is in the Aether program: the
script's address does nothing without an **upload code**, and you make the codes here, one
per station, each good for a few uploads.

## Setting it up (once, about ten minutes)

1. Open <https://script.google.com>, signed in to the account whose Drive should receive the
   files, and choose **New project**. Name it *Aether HF uploads*.
2. Replace everything in `Code.gs` with this folder's [`Code.gs`](Code.gs). At the top, check
   `OWNER_CALL` (your callsign) and `STATIONS` (the stations to make codes for).
3. **Project Settings** (the gear) → tick *Show "appsscript.json" manifest file in editor*. Back
   in the editor, replace `appsscript.json` with this folder's
   [`appsscript.json`](appsscript.json).
4. Pick **setup** in the function list beside *Run*, and press **Run**. Google asks you to
   authorise the script: *Review permissions* → your account → *Advanced* → *Go to Aether HF
   uploads (unsafe)* → *Allow*. Google shows that warning for any script nobody has submitted
   for review; this one is yours. The log shows the new folder, *Aether HF uploads*.
5. Pick **makeCodes** and **Run**. The log shows one code per station, such as
   `WC4Y: WC4Y-3f9a0c1d2e`. Codes are kept in the project; **listCodes** shows them again,
   with the uploads each has left.
6. **Deploy → New deployment** → the gear beside *Select type* → **Web app**. Set *Execute as*
   to **Me** and *Who has access* to **Anyone**. Press **Deploy** and copy the *Web app URL*.
   It ends in `/exec`.
7. Check it from a terminal in the Aether HF folder:

   ```
   python tools/drive_upload/try_upload.py https://script.google.com/macros/s/…/exec WC4Y-3f9a0c1d2e
   ```

   It sends a 1 MB test file the way a station does and prints its link. You get an email,
   and that code has one upload fewer. Delete the test file from the folder if you like.

## Asking a station for its files

On the panel's **Log** tab: **Send files…** → *Ask a station for its files*. Fill in the
station's email, which sessions you want and how far back, tick *audio* if you want the
recordings, and fill in the **upload address** (the `/exec` URL, which the panel remembers)
and **that station's code**. *Write request* opens an email to them with the link. When they
open the link, their panel shows **Send to KK4ODA**, and the files come to your folder.

## Sending your own files there

The same script takes your own files. Add your callsign to `STATIONS` and run **makeCodes**
for a code of your own. Then, on the **Log** tab, **Send files…** → *Send my files*: put the
upload address and your code in the **upload to** / **code** row (both remembered), and press
**Upload**. The zip goes to your folder as another station's would, and you get the email.
Another operator can give you their address and a code for your callsign, and **Upload** sends
there instead.

## Codes

A code is for one station. Make more with **makeCodes** (edit `STATIONS` first), and revoke
one with **revokeCode**: put the code between its quotes, then Run. To stop all uploads,
*Deploy → Manage deployments → Archive*. A new deployment gives a new address.

## What it allows

* Only a code you made, with uploads left, and only up to 400 MB a file (`MAX_BYTES`).
* Each upload goes into the folder under a name with the time, the station and the zip's
  name, and its description says who sent it with which code.
* The emails come from your own account to your own account (Apps Script's daily quota is
  100 for a personal account).
* The script runs as you, so it can reach your Drive: anybody with the address and a valid
  code can put a file into that one folder, and nothing more. It cannot read, list or delete
  anything.

How it works: `begin` asks Google Drive for a *resumable upload* session for one file in the
folder, using your account's authority, and hands the session's address to the station. The
station sends the zip there in 8 MB pieces, picking up where a dropped connection left off.
Then `finish` checks the file is in the folder, counts the upload against the code and emails
you. The station's side is `core/aetherd/src/upload.rs` (`share.upload` in the control API).
