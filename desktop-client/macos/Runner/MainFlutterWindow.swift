import Cocoa
import FlutterMacOS

class MainFlutterWindow: NSWindow {
  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

    if let delegate = NSApp.delegate as? AppDelegate {
      delegate.lifecycleChannel = FlutterMethodChannel(
        name: "framelean/lifecycle", binaryMessenger: flutterViewController.engine.binaryMessenger)
    }
    RegisterGeneratedPlugins(registry: flutterViewController)

    super.awakeFromNib()
  }
}
