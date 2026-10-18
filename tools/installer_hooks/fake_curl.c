/* A stand-in for Windows' curl.exe, to see what the installer's hooks ask the modem: logs
 * its command line to the file FAKE_CURL_LOG names. When FAKE_CURL_FREES names a file, it
 * deletes it — the modem stopping when asked, for the fake Restart Manager. */
#include <windows.h>
#include <stdio.h>

int main(void) {
  char path[1024];
  if (GetEnvironmentVariableA("FAKE_CURL_LOG", path, sizeof path)) {
    FILE *f = fopen(path, "a");
    if (f) {
      fprintf(f, "%s\n", GetCommandLineA());
      fclose(f);
    }
  }
  if (GetEnvironmentVariableA("FAKE_CURL_FREES", path, sizeof path)) DeleteFileA(path);
  return 0;
}
