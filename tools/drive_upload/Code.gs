/**
 * Aether HF — the upload script: where other stations' files land when you ask for them.
 *
 * It runs in your own Google account as a web app (README.md beside this file says how to set
 * it up). An Aether HF station that opens your request link and presses Send asks it for a
 * place to put one zip (`begin`); it answers with a Google Drive resumable upload session for
 * one file in your folder, made with your account's authority; the station sends the zip
 * there in pieces; and tells it it is done (`finish`), which emails you a link to the file.
 *
 * Nothing secret is in the Aether program or in the link: the script's address is no use
 * without a code, and a code is one you made here for one station — good for a number of
 * uploads you choose, up to a size, and revoked by deleting it.
 *
 * Protocol: the station POSTs JSON as text; the answer is JSON, {ok: true, …} or
 * {ok: false, error: "a sentence the panel shows"}.
 */

/** Your callsign: the panel tells the sender the files went to you. */
const OWNER_CALL = 'KK4ODA';

/** The stations `makeCodes` makes a code for, and how many uploads each code allows. */
const STATIONS = ['WC4Y', 'W4TGA'];
const UPLOADS_PER_CODE = 3;

/** The largest file accepted, bytes: a day of recordings with their audio is well under it. */
const MAX_BYTES = 400 * 1024 * 1024;

const PROPS = PropertiesService.getScriptProperties();

// ── run these from the editor ─────────────────────────────────────────

/** Once: makes the folder the files land in, and says what to do next. */
function setup() {
  let id = PROPS.getProperty('FOLDER_ID');
  if (!id) {
    id = DriveApp.createFolder('Aether HF uploads').getId();
    PROPS.setProperty('FOLDER_ID', id);
  }
  Logger.log('Files will land in https://drive.google.com/drive/folders/' + id);
  Logger.log('Emails go to ' + Session.getEffectiveUser().getEmail());
  Logger.log('Next: run makeCodes, then Deploy > New deployment > Web app, ' +
      'Execute as: Me, Who has access: Anyone.');
}

/** A code for each station in STATIONS, each good for UPLOADS_PER_CODE uploads. */
function makeCodes() {
  STATIONS.forEach((station) => Logger.log(station + ': ' + newCode(station, UPLOADS_PER_CODE)));
}

/** Every code, who it is for and how many uploads it has left. */
function listCodes() {
  const all = PROPS.getProperties();
  Object.keys(all).filter((k) => k.startsWith('code:')).forEach((k) => {
    const c = JSON.parse(all[k]);
    Logger.log(k.slice(5) + '  ' + c.station + '  ' + c.left + ' left  (made ' + c.created + ')');
  });
}

/** Revoke a code: put it between the quotes and run this. */
function revokeCode() {
  const code = '';
  PROPS.deleteProperty('code:' + code);
  Logger.log('Revoked ' + code);
}

function newCode(station, uploads) {
  const call = String(station || '').toUpperCase().replace(/[^A-Z0-9]/g, '');
  const code = call + '-' + Utilities.getUuid().replace(/-/g, '').slice(0, 10);
  PROPS.setProperty('code:' + code, JSON.stringify({
    station: call,
    left: uploads || 1,
    created: new Date().toISOString(),
  }));
  return code;
}

// ── the web app ───────────────────────────────────────────────────────

/** A browser opening the address sees that the script is there. */
function doGet() {
  return json({ok: true, service: 'aether-hf upload', owner: OWNER_CALL});
}

function doPost(e) {
  let asked;
  try {
    asked = JSON.parse(e.postData.contents);
  } catch (_) {
    return json({ok: false, error: 'That is not a request this upload script reads.'});
  }
  try {
    if (asked.action === 'begin') return json(begin(asked));
    if (asked.action === 'finish') return json(finish(asked));
    return json({ok: false, error: 'The upload script does not know that request.'});
  } catch (err) {
    return json({ok: false, error: 'The upload script failed: ' + err.message});
  }
}

/** A place for one file: a Drive resumable upload session in the folder. */
function begin(asked) {
  const code = String(asked.code || '');
  const known = codeOf(code);
  if (!known) return {ok: false, error: 'This upload code is not known: ask ' + OWNER_CALL + ' for a new request link.'};
  if (known.left <= 0) return {ok: false, error: 'This upload code has been used up: ask ' + OWNER_CALL + ' for a new request link.'};
  const size = Number(asked.size);
  if (!(size > 0 && size <= MAX_BYTES)) {
    return {ok: false, error: 'The file is ' + size + ' bytes; ' + OWNER_CALL + ' takes up to ' +
        Math.round(MAX_BYTES / 1048576) + ' MB.'};
  }
  const folderId = PROPS.getProperty('FOLDER_ID');
  if (!folderId) return {ok: false, error: 'The upload script is not set up yet (run setup).'};
  DriveApp.getFolderById(folderId); // fails plainly if the folder is gone
  const name = stamp() + ' ' + known.station + ' ' + safe(asked.name);
  const response = UrlFetchApp.fetch(
      'https://www.googleapis.com/upload/drive/v3/files?uploadType=resumable&fields=id', {
        method: 'post',
        contentType: 'application/json; charset=UTF-8',
        headers: {
          'Authorization': 'Bearer ' + ScriptApp.getOAuthToken(),
          'X-Upload-Content-Type': 'application/zip',
          'X-Upload-Content-Length': String(size),
        },
        payload: JSON.stringify({
          name: name,
          parents: [folderId],
          description: 'From ' + safe(asked.callsign) + ' (code ' + code + '), Aether HF ' +
              safe(asked.version),
        }),
        muteHttpExceptions: true,
      });
  const headers = response.getHeaders();
  const session = headers.Location || headers.location;
  if (response.getResponseCode() !== 200 || !session) {
    return {ok: false, error: 'Google Drive would not open an upload (' + response.getResponseCode() + ').'};
  }
  return {ok: true, upload_url: session, to: OWNER_CALL};
}

/** The file is there: count the upload against the code, and email the owner. */
function finish(asked) {
  const code = String(asked.code || '');
  const lock = LockService.getScriptLock();
  lock.waitLock(20000);
  try {
    const known = codeOf(code);
    if (!known) return {ok: false, error: 'This upload code is not known.'};
    const file = DriveApp.getFileById(String(asked.file_id || ''));
    const folderId = PROPS.getProperty('FOLDER_ID');
    let inFolder = false;
    const parents = file.getParents();
    while (parents.hasNext()) inFolder = inFolder || parents.next().getId() === folderId;
    if (!inFolder) return {ok: false, error: 'That file is not one of this script\'s uploads.'};
    known.left -= 1;
    PROPS.setProperty('code:' + code, JSON.stringify(known));
    const size = file.getSize();
    const note = String(asked.note || '').slice(0, 2000);
    MailApp.sendEmail(
        Session.getEffectiveUser().getEmail(),
        'Aether HF files from ' + safe(asked.callsign) + ' (' + Math.round(size / 1024) + ' kB)',
        safe(asked.callsign) + ' sent ' + file.getName() + ', ' + size + ' bytes' +
            (size === Number(asked.size) ? '' : ' (they said ' + asked.size + ')') + '.\n\n' +
            (note ? 'Their note:\n' + note + '\n\n' : '') +
            file.getUrl() + '\n\nCode ' + code + ' (' + known.station + ') has ' + known.left +
            ' upload(s) left.');
    return {ok: true, link: file.getUrl()};
  } finally {
    lock.releaseLock();
  }
}

// ── helpers ───────────────────────────────────────────────────────────

function codeOf(code) {
  if (!/^[A-Za-z0-9_-]{4,64}$/.test(code)) return null;
  const raw = PROPS.getProperty('code:' + code);
  return raw ? JSON.parse(raw) : null;
}

function safe(text) {
  return String(text || '?').replace(/[^A-Za-z0-9._\/-]/g, '_').slice(0, 120);
}

function stamp() {
  return Utilities.formatDate(new Date(), 'UTC', "yyyyMMdd-HHmmss'Z'");
}

function json(value) {
  return ContentService.createTextOutput(JSON.stringify(value))
      .setMimeType(ContentService.MimeType.JSON);
}
