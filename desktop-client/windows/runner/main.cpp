#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>
#include <flutter_windows.h>
#include <windows.h>

#include <algorithm>

#include "flutter_window.h"
#include "utils.h"

namespace {

constexpr unsigned int kDefaultWindowWidth = 760;
constexpr unsigned int kDefaultWindowHeight = 720;
constexpr double kWindowHeightRatio = 0.76;
constexpr double kWindowMargin = 32.0;
constexpr double kMaxWindowHeight = 920.0;

Win32Window::Size AdaptivePrimaryMonitorSize() {
  POINT primary_point = {0, 0};
  HMONITOR monitor = MonitorFromPoint(primary_point, MONITOR_DEFAULTTOPRIMARY);

  MONITORINFO monitor_info;
  monitor_info.cbSize = sizeof(MONITORINFO);
  if (!GetMonitorInfo(monitor, &monitor_info)) {
    return Win32Window::Size(kDefaultWindowWidth, kDefaultWindowHeight);
  }

  const UINT dpi = FlutterDesktopGetDpiForMonitor(monitor);
  const double scale_factor = dpi / 96.0;
  const RECT work_area = monitor_info.rcWork;
  const double logical_width =
      (work_area.right - work_area.left) / scale_factor;
  const double logical_height =
      (work_area.bottom - work_area.top) / scale_factor;
  const double usable_width = std::max(360.0, logical_width - kWindowMargin);
  const double usable_height = std::max(360.0, logical_height - kWindowMargin);
  const double target_width = static_cast<double>(kDefaultWindowWidth);
  const double target_height =
      std::clamp(logical_height * kWindowHeightRatio,
                 static_cast<double>(kDefaultWindowHeight), kMaxWindowHeight);

  return Win32Window::Size(
      static_cast<unsigned int>(std::min(target_width, usable_width)),
      static_cast<unsigned int>(std::min(target_height, usable_height)));
}

Win32Window::Point CenteredPrimaryMonitorOrigin(
    const Win32Window::Size& size) {
  POINT primary_point = {0, 0};
  HMONITOR monitor = MonitorFromPoint(primary_point, MONITOR_DEFAULTTOPRIMARY);

  MONITORINFO monitor_info;
  monitor_info.cbSize = sizeof(MONITORINFO);
  if (!GetMonitorInfo(monitor, &monitor_info)) {
    return Win32Window::Point(10, 10);
  }

  const UINT dpi = FlutterDesktopGetDpiForMonitor(monitor);
  const double scale_factor = dpi / 96.0;
  const LONG scaled_width = static_cast<LONG>(size.width * scale_factor);
  const LONG scaled_height = static_cast<LONG>(size.height * scale_factor);
  const RECT work_area = monitor_info.rcWork;
  const LONG work_width = work_area.right - work_area.left;
  const LONG work_height = work_area.bottom - work_area.top;
  const LONG centered_x =
      work_area.left + std::max<LONG>(0, work_width - scaled_width) / 2;
  const LONG centered_y =
      work_area.top + std::max<LONG>(0, work_height - scaled_height) / 2;

  return Win32Window::Point(
      static_cast<unsigned int>(std::max<LONG>(0, centered_x) / scale_factor),
      static_cast<unsigned int>(std::max<LONG>(0, centered_y) / scale_factor));
}

}  // namespace

int APIENTRY wWinMain(_In_ HINSTANCE instance, _In_opt_ HINSTANCE prev,
                      _In_ wchar_t *command_line, _In_ int show_command) {
  // Attach to console when present (e.g., 'flutter run') or create a
  // new console when running with a debugger.
  if (!::AttachConsole(ATTACH_PARENT_PROCESS) && ::IsDebuggerPresent()) {
    CreateAndAttachConsole();
  }

  // Initialize COM, so that it is available for use in the library and/or
  // plugins.
  ::CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);

  flutter::DartProject project(L"data");

  std::vector<std::string> command_line_arguments =
      GetCommandLineArguments();

  project.set_dart_entrypoint_arguments(std::move(command_line_arguments));

  FlutterWindow window(project);
  Win32Window::Size size = AdaptivePrimaryMonitorSize();
  Win32Window::Point origin = CenteredPrimaryMonitorOrigin(size);
  if (!window.Create(L"FrameLean", origin, size)) {
    return EXIT_FAILURE;
  }
  window.SetQuitOnClose(true);

  ::MSG msg;
  while (::GetMessage(&msg, nullptr, 0, 0)) {
    ::TranslateMessage(&msg);
    ::DispatchMessage(&msg);
  }

  ::CoUninitialize();
  return EXIT_SUCCESS;
}
