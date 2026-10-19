import UIKit
import Metal

/// Subclass this delegate and override start() to call your Rust entrypoint.
/// Compile this source and gpui_ios.h into each native application host.
open class GPUIAppDelegate: UIResponder, UIApplicationDelegate {
    open func start() {}
    public func application(_ application: UIApplication, didFinishLaunchingWithOptions options: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        start()
        return true
    }
    public func applicationDidBecomeActive(_ application: UIApplication) {
        gpui_ios_lifecycle(0)
        GPUIHost.windows.values.forEach { $0.view.isPaused = false }
    }
    public func applicationWillResignActive(_ application: UIApplication) { gpui_ios_lifecycle(1) }
    public func applicationDidEnterBackground(_ application: UIApplication) {
        GPUIHost.windows.values.forEach { $0.view.cancelTouches(); $0.view.isPaused = true }
        gpui_ios_lifecycle(2)
    }
    public func applicationWillEnterForeground(_ application: UIApplication) { gpui_ios_lifecycle(3) }
    public func applicationDidReceiveMemoryWarning(_ application: UIApplication) { gpui_ios_memory_warning() }
    public func applicationWillTerminate(_ application: UIApplication) {
        GPUIHost.windows.values.forEach { $0.view.stop() }
        gpui_ios_shutdown()
        GPUIHost.windows.removeAll()
    }
}

private struct GPUIWindow { let window: UIWindow; let view: GPUIView }
private enum GPUIHost { static var windows: [UInt64: GPUIWindow] = [:] }

private final class TextPosition: UITextPosition {
    let offset: Int
    init(_ offset: Int) { self.offset = offset; super.init() }
}
private final class TextRange: UITextRange {
    let range: NSRange
    init(_ range: NSRange) { self.range = range; super.init() }
    override var start: UITextPosition { TextPosition(range.location) }
    override var end: UITextPosition { TextPosition(NSMaxRange(range)) }
    override var isEmpty: Bool { range.length == 0 }
}

private struct InputSnapshot: Equatable {
    var text = ""
    var selection: NSRange?
    var marked: NSRange?
}

private final class AccessibleElement: UIAccessibilityElement {
    let windowID: UInt64
    let nodeID: UInt64
    var canClick = false
    var canFocus = false
    init(view: GPUIView, node: UInt64) {
        windowID = view.id
        nodeID = node
        super.init(accessibilityContainer: view.semanticContainer ?? view)
        isAccessibilityElement = true
    }
    override func accessibilityActivate() -> Bool {
        guard canClick else { return false }
        gpui_ios_accessibility_action(windowID, nodeID, false)
        return true
    }
    override func accessibilityElementDidBecomeFocused() {
        if canFocus { gpui_ios_accessibility_action(windowID, nodeID, true) }
    }
}

private final class GPUIView: UIView, UITextInput {
    let id: UInt64
    weak var semanticContainer: UIView?
    private var displayLink: CADisplayLink?
    var isPaused = false { didSet { displayLink?.isPaused = isPaused } }
    private var touchIDs: [ObjectIdentifier: (UInt64, CGPoint)] = [:]
    private var nextTouch: UInt64 = 1
    private var keyboardTarget: CGRect = .null
    private var keyboardFrom: CGFloat = 0
    private var keyboardTo: CGFloat = 0
    private var keyboardStart: CFTimeInterval = 0
    private var keyboardDuration: Double = 0
    private var keyboardCurve: Int = 0
    private var keyboardOverlap: CGFloat = 0
    private var semanticElements: [UInt64: AccessibleElement] = [:]
    private var cachedInput: InputSnapshot?
    var tokenizer: UITextInputTokenizer { UITextInputStringTokenizer(textInput: self) }
    weak var inputDelegate: UITextInputDelegate?
    var markedTextStyle: [NSAttributedString.Key: Any]?
    var selectionAffinity: UITextStorageDirection = .forward
    var autocorrectionType: UITextAutocorrectionType = .no
    var autocapitalizationType: UITextAutocapitalizationType = .none
    var spellCheckingType: UITextSpellCheckingType = .no
    var keyboardType: UIKeyboardType = .default
    var keyboardAppearance: UIKeyboardAppearance = .default
    var returnKeyType: UIReturnKeyType = .default
    var enablesReturnKeyAutomatically = false
    var isSecureTextEntry = false

    override class var layerClass: AnyClass { CAMetalLayer.self }
    // UIKit's UITextInput accessibility specialization must not replace our semantic container.
    override var isAccessibilityElement: Bool { get { false } set {} }
    override var accessibilityTraits: UIAccessibilityTraits { get { [] } set {} }
    override var canBecomeFirstResponder: Bool { true }
    init(id: UInt64) {
        self.id = id
        super.init(frame: UIScreen.main.bounds)
        isMultipleTouchEnabled = true
        isAccessibilityElement = false
        NotificationCenter.default.addObserver(self, selector: #selector(keyboardChanged(_:)), name: UIResponder.keyboardWillChangeFrameNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(voiceOverChanged), name: UIAccessibility.voiceOverStatusDidChangeNotification, object: nil)
    }
    required init?(coder: NSCoder) { fatalError("Use init(id:)") }
    deinit { NotificationCenter.default.removeObserver(self) }
    func start() {
        let link = CADisplayLink(target: self, selector: #selector(frameTick(_:)))
        link.add(to: .main, forMode: .common)
        displayLink = link
        layoutIfNeeded()
        voiceOverChanged()
    }
    func stop() { displayLink?.invalidate(); displayLink = nil }
    @objc private func frameTick(_ link: CADisplayLink) {
        if keyboardDuration > 0 {
            let t = min(1, max(0, (link.timestamp - keyboardStart) / keyboardDuration))
            let progress: Double
            switch keyboardCurve {
            case 1: progress = t * t
            case 2: progress = 1 - (1 - t) * (1 - t)
            case 3: progress = t
            default: progress = t * t * (3 - 2 * t)
            }
            keyboardOverlap = keyboardFrom + (keyboardTo - keyboardFrom) * progress
            if t == 1 { keyboardDuration = 0; keyboardOverlap = keyboardTo }
            pushInsets()
        }
        gpui_ios_frame(id)
    }
    override func layoutSubviews() {
        super.layoutSubviews()
        let scale = window?.screen.scale ?? UIScreen.main.scale
        layer.contentsScale = scale
        (layer as? CAMetalLayer)?.drawableSize = CGSize(width: bounds.width * scale, height: bounds.height * scale)
        gpui_ios_resize(id, Float(bounds.width), Float(bounds.height), Float(scale), traitCollection.userInterfaceStyle == .dark)
        keyboardTo = overlap(of: keyboardTarget)
        if keyboardDuration == 0 { keyboardOverlap = keyboardTo }
        pushInsets()
    }
    override func safeAreaInsetsDidChange() { super.safeAreaInsetsDidChange(); pushInsets() }
    override func traitCollectionDidChange(_ previousTraitCollection: UITraitCollection?) { super.traitCollectionDidChange(previousTraitCollection); setNeedsLayout() }
    private func overlap(of screenFrame: CGRect) -> CGFloat {
        guard !screenFrame.isNull else { return 0 }
        let rect = convert(screenFrame, from: nil).intersection(bounds)
        // Floating/split keyboards must not become a fictitious full-width bottom inset.
        guard !rect.isNull, rect.maxY >= bounds.maxY - 1, rect.width >= bounds.width * 0.8 else { return 0 }
        return rect.height
    }
    @objc private func keyboardChanged(_ notification: Notification) {
        guard let data = notification.userInfo,
              let target = data[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect else { return }
        keyboardTarget = target
        keyboardFrom = keyboardOverlap
        keyboardTo = overlap(of: target)
        keyboardStart = CACurrentMediaTime()
        keyboardDuration = data[UIResponder.keyboardAnimationDurationUserInfoKey] as? Double ?? 0
        keyboardCurve = data[UIResponder.keyboardAnimationCurveUserInfoKey] as? Int ?? 0
        if keyboardDuration == 0 { keyboardOverlap = keyboardTo; pushInsets() }
    }
    private func pushInsets() {
        let inset = safeAreaInsets
        gpui_ios_insets(id, Float(inset.top), Float(inset.right), Float(inset.bottom), Float(inset.left), Float(keyboardOverlap))
    }
    private func touches(_ touches: Set<UITouch>, phase: Int32) {
        for touch in touches {
            let key = ObjectIdentifier(touch)
            let position = touch.location(in: self)
            if phase == 0 { touchIDs[key] = (nextTouch, position); nextTouch += 1 }
            guard let (number, _) = touchIDs[key] else { continue }
            touchIDs[key] = (number, position)
            let force = touch.maximumPossibleForce > 0 ? Float(touch.force / touch.maximumPossibleForce) : -1
            gpui_ios_touch(id, number, phase, Float(position.x), Float(position.y), force)
            if phase >= 2 { touchIDs.removeValue(forKey: key) }
        }
    }
    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) { self.touches(touches, phase: 0) }
    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) { self.touches(touches, phase: 1) }
    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) { self.touches(touches, phase: 2) }
    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) { self.touches(touches, phase: 3) }
    func cancelTouches() {
        for (_, (number, point)) in touchIDs { gpui_ios_touch(id, number, 3, Float(point.x), Float(point.y), -1) }
        touchIDs.removeAll()
    }
    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        var unhandled: Set<UIPress> = []
        for press in presses {
            guard let key = press.key else { unhandled.insert(press); continue }
            // Candidate navigation and confirmation belong to UIKit while composing.
            if markedTextRange != nil && key.modifierFlags.intersection([.command, .control]).isEmpty {
                unhandled.insert(press)
                continue
            }
            let name: String?
            switch key.keyCode {
            case .keyboardLeftArrow: name = "left"
            case .keyboardRightArrow: name = "right"
            case .keyboardUpArrow: name = "up"
            case .keyboardDownArrow: name = "down"
            case .keyboardEscape: name = "escape"
            case .keyboardTab: name = "tab"
            case .keyboardReturnOrEnter: name = "enter"
            default: name = key.modifierFlags.intersection([.command, .control]).isEmpty ? nil : key.charactersIgnoringModifiers
            }
            if let name {
                var flags: UInt32 = 0
                if key.modifierFlags.contains(.shift) { flags |= 1 }
                if key.modifierFlags.contains(.control) { flags |= 2 }
                if key.modifierFlags.contains(.alternate) { flags |= 4 }
                if key.modifierFlags.contains(.command) { flags |= 8 }
                name.withCString { gpui_ios_key(id, $0, flags) }
            } else { unhandled.insert(press) }
        }
        if !unhandled.isEmpty { super.pressesBegan(unhandled, with: event) }
    }

    private func readSnapshot() -> InputSnapshot {
        guard let pointer = gpui_ios_text_snapshot(id) else { return InputSnapshot() }
        defer { gpui_ios_string_free(pointer) }
        guard let data = String(cString: pointer).data(using: .utf8),
              let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return InputSnapshot() }
        func range(_ key: String) -> NSRange? {
            guard let values = value[key] as? [Int], values.count == 2, values[0] >= 0, values[1] >= values[0] else { return nil }
            return NSRange(location: values[0], length: values[1] - values[0])
        }
        return InputSnapshot(text: value["text"] as? String ?? "", selection: range("selection"), marked: range("marked"))
    }
    private var snapshot: InputSnapshot {
        if let value = cachedInput { return value }
        let value = readSnapshot()
        cachedInput = value
        return value
    }
    private var document: NSString { snapshot.text as NSString }
    var hasText: Bool { document.length > 0 }
    var beginningOfDocument: UITextPosition { TextPosition(0) }
    var endOfDocument: UITextPosition { TextPosition(document.length) }
    var selectedTextRange: UITextRange? {
        get { snapshot.selection.map(TextRange.init) }
        set {
            guard let range = newValue as? TextRange else { return }
            gpui_ios_set_selection(id, UInt64(range.range.location), UInt64(NSMaxRange(range.range)))
            cachedInput = readSnapshot()
        }
    }
    var markedTextRange: UITextRange? { snapshot.marked.map(TextRange.init) }
    func insertText(_ text: String) {
        text.withCString { gpui_ios_replace_text(id, -1, -1, $0, false, -1, -1) }
        cachedInput = readSnapshot()
    }
    func deleteBackward() {
        gpui_ios_delete_backward(id)
        cachedInput = readSnapshot()
    }
    func text(in range: UITextRange) -> String? {
        guard let range = range as? TextRange else { return nil }
        let text = document
        guard NSMaxRange(range.range) <= text.length else { return nil }
        return text.substring(with: range.range)
    }
    func replace(_ range: UITextRange, withText text: String) {
        guard let range = range as? TextRange else { return }
        text.withCString { gpui_ios_replace_text(id, Int64(range.range.location), Int64(NSMaxRange(range.range)), $0, false, -1, -1) }
        cachedInput = readSnapshot()
    }
    func setMarkedText(_ markedText: String?, selectedRange: NSRange) {
        let text = markedText ?? ""
        text.withCString { gpui_ios_replace_text(id, -1, -1, $0, true, Int64(selectedRange.location), Int64(NSMaxRange(selectedRange))) }
        cachedInput = readSnapshot()
    }
    func unmarkText() { gpui_ios_unmark(id); cachedInput = readSnapshot() }
    func textRange(from fromPosition: UITextPosition, to toPosition: UITextPosition) -> UITextRange? {
        guard let from = fromPosition as? TextPosition, let to = toPosition as? TextPosition else { return nil }
        return TextRange(NSRange(location: min(from.offset, to.offset), length: abs(to.offset - from.offset)))
    }
    func position(from position: UITextPosition, offset: Int) -> UITextPosition? {
        guard let position = position as? TextPosition else { return nil }
        let (value, overflow) = position.offset.addingReportingOverflow(offset)
        guard !overflow, value >= 0, value <= document.length else { return nil }
        return TextPosition(value)
    }
    func compare(_ position: UITextPosition, to other: UITextPosition) -> ComparisonResult {
        guard let a = position as? TextPosition, let b = other as? TextPosition else { return .orderedSame }
        return a.offset == b.offset ? .orderedSame : (a.offset < b.offset ? .orderedAscending : .orderedDescending)
    }
    func offset(from: UITextPosition, to: UITextPosition) -> Int { ((to as? TextPosition)?.offset ?? 0) - ((from as? TextPosition)?.offset ?? 0) }
    func position(from position: UITextPosition, in direction: UITextLayoutDirection, offset: Int) -> UITextPosition? {
        guard let start = position as? TextPosition, offset != Int.min else { return nil }
        let resolvedDirection: UITextLayoutDirection
        if offset >= 0 { resolvedDirection = direction }
        else {
            switch direction {
            case .left: resolvedDirection = .right
            case .right: resolvedDirection = .left
            case .up: resolvedDirection = .down
            case .down: resolvedDirection = .up
            @unknown default: return nil
            }
        }
        var current = start.offset
        for _ in 0..<abs(offset) {
            guard let next = visualNeighbor(current, direction: resolvedDirection) else { return nil }
            current = next
        }
        return TextPosition(current)
    }
    private func visualNeighbor(_ offset: Int, direction: UITextLayoutDirection) -> Int? {
        let text = document
        let current = caretRect(for: TextPosition(offset))
        guard current.height > 0 else { return nil }
        var boundaries = [0]
        var index = 0
        while index < text.length {
            let range = text.rangeOfComposedCharacterSequence(at: index)
            index = NSMaxRange(range)
            boundaries.append(index)
        }
        // Consult the GPUI shaped caret geometry; never equate logical order to left/right.
        return boundaries.filter { $0 != offset }.compactMap { value -> (Int, CGFloat)? in
            let rect = caretRect(for: TextPosition(value))
            let dx = rect.midX - current.midX, dy = rect.midY - current.midY
            switch direction {
            case .left: guard dx < -0.1 && abs(dy) < current.height / 2 else { return nil }; return (value, -dx)
            case .right: guard dx > 0.1 && abs(dy) < current.height / 2 else { return nil }; return (value, dx)
            case .up: guard dy < -current.height / 2 else { return nil }; return (value, -dy * 10000 + abs(dx))
            case .down: guard dy > current.height / 2 else { return nil }; return (value, dy * 10000 + abs(dx))
            @unknown default: return nil
            }
        }.min { $0.1 < $1.1 }?.0
    }
    func position(within range: UITextRange, farthestIn direction: UITextLayoutDirection) -> UITextPosition? {
        guard let range = range as? TextRange else { return nil }
        let a = TextPosition(range.range.location), b = TextPosition(NSMaxRange(range.range))
        let first = caretRect(for: a), last = caretRect(for: b)
        switch direction {
        case .left: return first.minX <= last.minX ? a : b
        case .right: return first.maxX >= last.maxX ? a : b
        case .up: return first.minY <= last.minY ? a : b
        case .down: return first.maxY >= last.maxY ? a : b
        @unknown default: return nil
        }
    }
    func characterRange(byExtending position: UITextPosition, in direction: UITextLayoutDirection) -> UITextRange? {
        guard let neighbor = self.position(from: position, in: direction, offset: 1) else { return nil }
        return textRange(from: position, to: neighbor)
    }
    func baseWritingDirection(for position: UITextPosition, in direction: UITextStorageDirection) -> NSWritingDirection { .natural }
    func setBaseWritingDirection(_ writingDirection: NSWritingDirection, for range: UITextRange) { }
    func firstRect(for range: UITextRange) -> CGRect {
        guard let range = range as? TextRange else { return .zero }
        let rect = gpui_ios_text_rect(id, UInt64(range.range.location), UInt64(NSMaxRange(range.range)))
        return CGRect(x: rect.x, y: rect.y, width: rect.width, height: rect.height)
    }
    func caretRect(for position: UITextPosition) -> CGRect {
        guard let position = position as? TextPosition else { return .zero }
        let rect = gpui_ios_text_rect(id, UInt64(position.offset), UInt64(position.offset))
        return CGRect(x: rect.x, y: rect.y, width: max(1, rect.width), height: rect.height)
    }
    // Stage 1: GPUI Kit paints text selection; UIKit is the IME client only.
    func selectionRects(for range: UITextRange) -> [UITextSelectionRect] { [] }
    func closestPosition(to point: CGPoint) -> UITextPosition? {
        let index = gpui_ios_text_index(id, Float(point.x), Float(point.y))
        return index >= 0 ? TextPosition(Int(index)) : nil
    }
    func closestPosition(to point: CGPoint, within range: UITextRange) -> UITextPosition? {
        guard let position = closestPosition(to: point) as? TextPosition, let range = range as? TextRange else { return nil }
        return TextPosition(min(NSMaxRange(range.range), max(range.range.location, position.offset)))
    }
    func characterRange(at point: CGPoint) -> UITextRange? {
        guard let position = closestPosition(to: point) as? TextPosition, position.offset < document.length else { return nil }
        return TextRange(document.rangeOfComposedCharacterSequence(at: position.offset))
    }
    func notifyTextChanged() {
        let next = readSnapshot()
        let previous = snapshot
        guard previous != next else { return }
        let textChanged = previous.text != next.text || previous.marked != next.marked
        let selectionChanged = previous.selection != next.selection
        // UIKit queries during willChange see the old state, then the new state during didChange.
        if textChanged { inputDelegate?.textWillChange(self) }
        if selectionChanged { inputDelegate?.selectionWillChange(self) }
        cachedInput = next
        if textChanged { inputDelegate?.textDidChange(self) }
        if selectionChanged { inputDelegate?.selectionDidChange(self) }
    }
    @objc func voiceOverChanged() {
        // UI automation can request the same semantic tree without changing VoiceOver settings.
        let requested = ProcessInfo.processInfo.arguments.contains("--gpui-ios-accessibility")
        gpui_ios_accessibility_enabled(id, UIAccessibility.isVoiceOverRunning || requested)
    }
    func updateAccessibility(_ data: Data) {
        guard let nodes = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else { return }
        var next: [UInt64: AccessibleElement] = [:]
        var ordered: [AccessibleElement] = []
        for node in nodes {
            guard let stringID = node["id"] as? String, let nodeID = UInt64(stringID),
                  let bounds = node["bounds"] as? [Double], bounds.count == 4 else { continue }
            let element = semanticElements[nodeID] ?? AccessibleElement(view: self, node: nodeID)
            element.accessibilityLabel = node["label"] as? String
            element.accessibilityValue = node["value"] as? String
            element.canClick = node["click"] as? Bool ?? false
            element.canFocus = node["focus"] as? Bool ?? false
            var traits: UIAccessibilityTraits = []
            switch node["role"] as? String {
            case "Button": traits.insert(.button)
            case "Link": traits.insert(.link)
            case "Image": traits.insert(.image)
            case "Heading": traits.insert(.header)
            case "Label", "StaticText": traits.insert(.staticText)
            default: break
            }
            if node["disabled"] as? Bool == true { traits.insert(.notEnabled) }
            element.accessibilityTraits = traits
            element.accessibilityFrameInContainerSpace = CGRect(x: bounds[0], y: bounds[1], width: bounds[2], height: bounds[3])
            if element.accessibilityLabel != nil || element.canClick || element.canFocus {
                ordered.append(element)
                next[nodeID] = element
            }
        }
        let changed = Set(next.keys) != Set(semanticElements.keys)
        semanticElements = next
        let container = semanticContainer ?? self
        container.accessibilityElements = ordered
        if #available(iOS 17.0, *) { container.automationElements = ordered }
        if changed && UIAccessibility.isVoiceOverRunning { UIAccessibility.post(notification: .layoutChanged, argument: nil) }
    }
}

@_cdecl("gpui_ios_host_create")
func hostCreate(_ id: UInt64) -> UnsafeMutableRawPointer? {
    precondition(Thread.isMainThread)
    let window = UIWindow(frame: UIScreen.main.bounds)
    let view = GPUIView(id: id)
    // UITextInput is exposed by UIKit as a TextView regardless of custom children.
    // A plain parent vends the GPUI tree and hides that IME implementation detail.
    let container = UIView(frame: UIScreen.main.bounds)
    container.isAccessibilityElement = false
    view.semanticContainer = container
    view.accessibilityElementsHidden = true
    view.frame = container.bounds
    view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
    container.addSubview(view)
    let controller = UIViewController()
    controller.view = container
    window.rootViewController = controller
    GPUIHost.windows[id] = GPUIWindow(window: window, view: view)
    return Unmanaged.passUnretained(view).toOpaque()
}
private func view(_ pointer: UnsafeMutableRawPointer) -> GPUIView { Unmanaged<GPUIView>.fromOpaque(pointer).takeUnretainedValue() }
@_cdecl("gpui_ios_host_layer") func hostLayer(_ pointer: UnsafeMutableRawPointer) -> UnsafeMutableRawPointer { Unmanaged.passUnretained(view(pointer).layer).toOpaque() }
@_cdecl("gpui_ios_host_show") func hostShow(_ pointer: UnsafeMutableRawPointer) {
    let view = view(pointer)
    DispatchQueue.main.async { GPUIHost.windows[view.id]?.window.makeKeyAndVisible(); view.start() }
}
@_cdecl("gpui_ios_host_destroy") func hostDestroy(_ pointer: UnsafeMutableRawPointer) {
    let id = view(pointer).id
    DispatchQueue.main.async { GPUIHost.windows[id]?.view.stop(); GPUIHost.windows.removeValue(forKey: id) }
}
@_cdecl("gpui_ios_host_screen") func hostScreen(_ width: UnsafeMutablePointer<Float>, _ height: UnsafeMutablePointer<Float>, _ scale: UnsafeMutablePointer<Float>) {
    width.pointee = Float(UIScreen.main.bounds.width); height.pointee = Float(UIScreen.main.bounds.height); scale.pointee = Float(UIScreen.main.scale)
}
@_cdecl("gpui_ios_host_keyboard") func hostKeyboard(_ pointer: UnsafeMutableRawPointer, _ visible: Bool) {
    let view = view(pointer)
    DispatchQueue.main.async { if visible { view.becomeFirstResponder() } else { view.resignFirstResponder() } }
}
@_cdecl("gpui_ios_host_text_changed") func hostTextChanged(_ pointer: UnsafeMutableRawPointer) {
    let view = view(pointer)
    DispatchQueue.main.async { view.notifyTextChanged() }
}
@_cdecl("gpui_ios_host_text_configuration") func hostTextConfiguration(_ pointer: UnsafeMutableRawPointer, _ autocorrect: Bool, _ capitalize: Int32, _ suggestions: Bool) {
    let view = view(pointer)
    DispatchQueue.main.async {
        view.autocorrectionType = autocorrect ? .yes : .no
        view.autocapitalizationType = UITextAutocapitalizationType(rawValue: Int(capitalize)) ?? .none
        view.spellCheckingType = suggestions ? .yes : .no
        if view.isFirstResponder { view.reloadInputViews() }
    }
}
@_cdecl("gpui_ios_host_open_url") func hostOpenURL(_ text: UnsafePointer<CChar>) { if let url = URL(string: String(cString: text)) { UIApplication.shared.open(url) } }
@_cdecl("gpui_ios_host_clipboard_read") func hostClipboardRead() -> UnsafeMutablePointer<CChar>? { UIPasteboard.general.string.flatMap { strdup($0) } }
@_cdecl("gpui_ios_host_clipboard_write") func hostClipboardWrite(_ text: UnsafePointer<CChar>) { UIPasteboard.general.string = String(cString: text) }
@_cdecl("gpui_ios_host_free_string") func hostFreeString(_ text: UnsafeMutablePointer<CChar>) { free(text) }
@_cdecl("gpui_ios_host_data_directory") func hostDataDirectory() -> UnsafeMutablePointer<CChar>? {
    guard let directory = try? FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true) else { return nil }
    return strdup(directory.path)
}
@_cdecl("gpui_ios_host_accessibility") func hostAccessibility(_ pointer: UnsafeMutableRawPointer, _ json: UnsafePointer<CChar>) {
    guard let data = String(cString: json).data(using: .utf8) else { return }
    let view = view(pointer)
    DispatchQueue.main.async { view.updateAccessibility(data) }
}
