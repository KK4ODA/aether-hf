/* A stand-in for Windows' Restart Manager (rstrtmgr.dll), to drive the installer's hooks
 * (app/src-tauri/windows/hooks.nsh) under Wine, whose own is a stub that reports nothing.
 *
 * A registered file P is held while both P and P.held exist; P.held says how many processes
 * hold it (1 when empty), named "Fake Holder <n>" with process ids 4240 + n. RmShutdown lets
 * go of it when P.closes exists, after the milliseconds P.closes says, and otherwise fails
 * as Windows does for a process it cannot close. Every call is logged, with GetTickCount, to
 * the file FAKE_RM_LOG names, so a test can tell what was asked and when. */
#include <windows.h>
#include <stdarg.h>
#include <stdio.h>
#include <wchar.h>

typedef struct {
  DWORD dwProcessId;
  FILETIME ProcessStartTime;
} RM_UNIQUE_PROCESS;

typedef struct {
  RM_UNIQUE_PROCESS Process;
  WCHAR strAppName[256];
  WCHAR strServiceShortName[64];
  DWORD ApplicationType;
  ULONG AppStatus;
  DWORD TSSessionId;
  BOOL bRestartable;
} RM_PROCESS_INFO;

/* the size the hooks step through the list by (AETHER_RM_PROCESS_INFO) */
_Static_assert(sizeof(RM_PROCESS_INFO) == 668, "RM_PROCESS_INFO is 668 bytes");

static WCHAR registered[1024];

static void say(const char *fmt, ...) {
  char path[1024];
  if (!GetEnvironmentVariableA("FAKE_RM_LOG", path, sizeof path)) return;
  FILE *f = fopen(path, "a");
  if (!f) return;
  fprintf(f, "%lu ", (unsigned long)GetTickCount());
  va_list ap;
  va_start(ap, fmt);
  vfprintf(f, fmt, ap);
  va_end(ap);
  fputc('\n', f);
  fclose(f);
}

static BOOL exists(const WCHAR *path) {
  return GetFileAttributesW(path) != INVALID_FILE_ATTRIBUTES;
}

static int holders(void) {
  WCHAR held[1100];
  if (!registered[0] || !exists(registered)) return 0;
  swprintf(held, 1100, L"%ls.held", registered);
  if (!exists(held)) return 0;
  int n = 1;
  FILE *f = _wfopen(held, L"r");
  if (f) {
    if (fscanf(f, "%d", &n) != 1) n = 1;
    fclose(f);
  }
  return n;
}

DWORD WINAPI RmStartSession(DWORD *handle, DWORD flags, WCHAR *key) {
  *handle = 7;
  if (key) key[0] = 0;
  registered[0] = 0;
  say("start");
  return 0;
}

DWORD WINAPI RmEndSession(DWORD handle) {
  say("end %lu", (unsigned long)handle);
  return 0;
}

DWORD WINAPI RmRegisterResources(DWORD handle, UINT nfiles, LPCWSTR files[], UINT napps,
                                 RM_UNIQUE_PROCESS apps[], UINT nsvcs, LPCWSTR svcs[]) {
  if (nfiles > 0) wcsncpy(registered, files[0], 1023);
  say("register %ls", registered);
  return 0;
}

DWORD WINAPI RmGetList(DWORD handle, UINT *needed, UINT *count, RM_PROCESS_INFO *apps,
                       DWORD *reasons) {
  int n = holders();
  UINT room = *count;
  *needed = n;
  if (reasons) *reasons = 0;
  say("getlist room=%u holders=%d buffer=%s", room, n, apps ? "yes" : "no");
  if (n == 0) {
    *count = 0;
    return 0;
  }
  if (room < (UINT)n || !apps) {
    *count = 0;
    return ERROR_MORE_DATA;
  }
  for (int i = 0; i < n; i++) {
    memset(&apps[i], 0, sizeof apps[i]);
    apps[i].Process.dwProcessId = 4241 + i;
    apps[i].Process.ProcessStartTime.dwLowDateTime = 0xDEADBEEF;
    apps[i].Process.ProcessStartTime.dwHighDateTime = 0xFEEDFACE;
    swprintf(apps[i].strAppName, 256, L"Fake Holder %d", i + 1);
    wcscpy(apps[i].strServiceShortName, L"fakesvc");
    apps[i].ApplicationType = 5;
  }
  *count = n;
  return 0;
}

/* lets go of a file after the delay its .closes names; arg is the .held path, freed here */
static DWORD WINAPI closer(void *arg) {
  WCHAR *held = arg, closes[1100];
  DWORD ms = 0;
  wcscpy(closes, held);
  closes[wcslen(closes) - 5] = 0; /* less ".held" */
  wcscat(closes, L".closes");
  FILE *f = _wfopen(closes, L"r");
  if (f) {
    unsigned long v;
    if (fscanf(f, "%lu", &v) == 1) ms = v;
    fclose(f);
  }
  Sleep(ms);
  DeleteFileW(held);
  say("closed %ls", held);
  free(held);
  return 0;
}

DWORD WINAPI RmShutdown(DWORD handle, ULONG flags, void *status) {
  WCHAR closes[1100];
  WCHAR *held = malloc(1100 * sizeof(WCHAR));
  swprintf(closes, 1100, L"%ls.closes", registered);
  swprintf(held, 1100, L"%ls.held", registered);
  say("shutdown flags=%lu %ls closes=%d", flags, registered, exists(closes));
  if (exists(closes)) {
    CloseHandle(CreateThread(NULL, 0, closer, held, 0, NULL));
  } else {
    free(held);
  }
  return holders() ? ERROR_FAIL_SHUTDOWN : 0;
}
