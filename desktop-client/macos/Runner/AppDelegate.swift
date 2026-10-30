import Cocoa
import FlutterMacOS

@main
class AppDelegate: FlutterAppDelegate {
  var lifecycleChannel: FlutterMethodChannel?
  override func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
  override func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
    // Closing the last window only disconnects the UI; menu Quit requests coordinated shutdown.
    if !sender.windows.contains(where: { $0.isVisible }) { return .terminateNow }
    guard let channel = lifecycleChannel else { return .terminateNow }
    channel.invokeMethod("requestExit", arguments: nil) { _ in
      sender.reply(toApplicationShouldTerminate: false)
    }
    return .terminateLater
  }
  override func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool { true }
}
