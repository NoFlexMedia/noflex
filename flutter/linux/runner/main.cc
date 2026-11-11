// SPDX-License-Identifier: AGPL-3.0-only
#include <unistd.h>

#include <cstdio>

#include "my_application.h"

int main(int argc, char** argv) {
  int status;
  {
    g_autoptr(MyApplication) app = my_application_new();
    status = g_application_run(G_APPLICATION(app), argc, argv);
  }
  // NVIDIA's EGL driver can crash in its exit handlers when GL contexts outlive
  // the window, after everything worth keeping is written. Leaving without
  // running them skips the crash and its core dump.
  fflush(nullptr);
  _exit(status);
}
