use anyhow::{Result, anyhow, bail};
use futures::channel::oneshot;
use gpui::*;
use gpui_apple::{
    AppleDispatcher, AppleTextSystem,
    metal_renderer::{InstanceBufferPool, MetalRenderer},
};
use parking_lot::Mutex;
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, UiKitWindowHandle, WindowHandle,
};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    ffi::{CString, OsString, c_void},
    path::{Path, PathBuf},
    ptr::NonNull,
    rc::Rc,
    sync::Arc,
    time::Duration,
};
use uuid::Uuid;

pub(crate) type Callback<T> = RefCell<Option<T>>;
thread_local! {
    pub(crate) static WINDOWS: RefCell<BTreeMap<u64, Rc<WindowState>>> = const { RefCell::new(BTreeMap::new()) };
    pub(crate) static PLATFORM: RefCell<Option<Rc<IosPlatform>>> = const { RefCell::new(None) };
    static APPLICATION: RefCell<Option<ApplicationHandle>> = const { RefCell::new(None) };
}

/// Constructs an application hosted by `GPUIHost.swift` on UIKit's main thread.
pub fn application() -> Application {
    let platform = Rc::new(IosPlatform::new());
    PLATFORM.with(|slot| *slot.borrow_mut() = Some(platform.clone()));
    Application::with_platform(platform)
}

/// Launch and retain an embedded application until [`shutdown`].
pub fn run(launch: impl FnOnce(&mut App) + 'static) {
    run_with(application(), launch);
}

/// Retain a configured embedded application, including its asset source and HTTP client.
pub fn run_with(application: Application, launch: impl FnOnce(&mut App) + 'static) {
    let app = application.run_embedded(launch);
    APPLICATION.with(|slot| *slot.borrow_mut() = Some(app));
}

/// Release the application after the native host has stopped delivering events.
pub fn shutdown() {
    APPLICATION.with(|slot| slot.borrow_mut().take());
    WINDOWS.with(|windows| windows.borrow_mut().clear());
    PLATFORM.with(|slot| slot.borrow_mut().take());
}

/// The app's sandboxed Application Support directory, created by the native host.
pub fn data_directory() -> Result<PathBuf> {
    crate::bridge::take_host_string(unsafe { gpui_ios_host_data_directory() })
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("Application Support directory unavailable"))
}

/// Native iOS platform. UIKit creates views through the small C host ABI.
pub struct IosPlatform {
    background: BackgroundExecutor,
    foreground: ForegroundExecutor,
    text: Arc<AppleTextSystem>,
    pub(crate) lifecycle: Callback<Box<dyn FnMut(AppLifecyclePhase)>>,
    pub(crate) memory_warning: Callback<Box<dyn FnMut()>>,
    urls: Callback<Box<dyn FnMut(Vec<String>)>>,
    quit: Callback<Box<dyn FnMut() -> bool>>,
    next_id: Cell<u64>,
    clipboard: RefCell<Option<ClipboardItem>>,
}

impl Default for IosPlatform {
    fn default() -> Self {
        Self::new()
    }
}
impl IosPlatform {
    pub fn new() -> Self {
        let dispatcher = Arc::new(AppleDispatcher::new());
        Self {
            background: BackgroundExecutor::new(dispatcher.clone()),
            foreground: ForegroundExecutor::new(dispatcher),
            text: Arc::new(AppleTextSystem::new()),
            lifecycle: RefCell::new(None),
            memory_warning: RefCell::new(None),
            urls: RefCell::new(None),
            quit: RefCell::new(None),
            next_id: Cell::new(1),
            clipboard: RefCell::new(None),
        }
    }
}

#[derive(Debug)]
struct IosDisplay;
impl PlatformDisplay for IosDisplay {
    fn id(&self) -> DisplayId {
        DisplayId::new(1)
    }
    fn uuid(&self) -> Result<Uuid> {
        Ok(Uuid::from_u128(1))
    }
    fn bounds(&self) -> Bounds<Pixels> {
        let mut width = 0.;
        let mut height = 0.;
        let mut scale = 1.;
        // SAFETY: the host writes these stack values synchronously on the main thread.
        unsafe { gpui_ios_host_screen(&mut width, &mut height, &mut scale) };
        Bounds::new(Point::default(), size(px(width), px(height)))
    }
    fn refresh_interval(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(1. / 60.))
    }
}

struct IosKeyboard;
impl PlatformKeyboardLayout for IosKeyboard {
    fn id(&self) -> &str {
        "ios"
    }
    fn name(&self) -> &str {
        "iOS keyboard"
    }
}

impl Platform for IosPlatform {
    fn platform_info(&self) -> PlatformInfo {
        PlatformInfo {
            os: OperatingSystem::Ios,
            target: PlatformTarget::Native,
        }
    }

    fn background_executor(&self) -> BackgroundExecutor {
        self.background.clone()
    }
    fn foreground_executor(&self) -> ForegroundExecutor {
        self.foreground.clone()
    }
    fn text_system(&self) -> Arc<dyn PlatformTextSystem> {
        self.text.clone()
    }
    fn run(&self, launch: Box<dyn FnOnce()>) {
        launch();
    }
    fn quit(&self) {
        if let Some(callback) = self.quit.borrow_mut().as_mut() {
            callback();
        }
    }
    fn restart(&self, _: Option<PathBuf>, _: Vec<OsString>) {
        log::warn!("iOS does not support process restart");
    }
    fn activate(&self, _: bool) {}
    fn hide(&self) {}
    fn hide_other_apps(&self) {}
    fn unhide_other_apps(&self) {}
    fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
        vec![Rc::new(IosDisplay)]
    }
    fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        Some(Rc::new(IosDisplay))
    }
    fn on_displays_changed(&self, _: Box<dyn FnMut()>) {}
    fn active_window(&self) -> Option<AnyWindowHandle> {
        WINDOWS.with(|windows| {
            windows
                .borrow()
                .values()
                .find(|w| w.active.get())
                .map(|w| w.handle)
        })
    }
    fn open_window(
        &self,
        handle: AnyWindowHandle,
        _: WindowParams,
    ) -> Result<Box<dyn PlatformWindow>> {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        // SAFETY: UIKit owns and retains the returned UIView until host_destroy.
        let view = NonNull::new(unsafe { gpui_ios_host_create(id) })
            .ok_or_else(|| anyhow!("UIKit could not create a view"))?;
        let layer = unsafe { gpui_ios_host_layer(view.as_ptr()) };
        if layer.is_null() {
            bail!("UIKit did not supply a CAMetalLayer");
        }
        // SAFETY: host_layer returns the live backing CAMetalLayer of this retained view.
        let renderer = MetalRenderer::from_layer(
            Arc::new(Mutex::new(InstanceBufferPool::default())),
            unsafe { &*layer.cast() },
            false,
        );
        let state = Rc::new(WindowState {
            id,
            handle,
            view,
            renderer: RefCell::new(renderer),
            bounds: Cell::new(IosDisplay.bounds()),
            scale: Cell::new(1.),
            active: Cell::new(true),
            visible: Cell::new(true),
            appearance: Cell::new(WindowAppearance::Light),
            background: Cell::new(WindowBackgroundAppearance::Opaque),
            title: RefCell::new(String::new()),
            insets: RefCell::new(WindowInsets::default()),
            input_handler: RefCell::new(None),
            frame: RefCell::new(None),
            input: RefCell::new(None),
            active_changed: RefCell::new(None),
            visibility_changed: RefCell::new(None),
            resized: RefCell::new(None),
            appearance_changed: RefCell::new(None),
            inset_changed: RefCell::new(None),
            viewport_changed: RefCell::new(None),
            close: RefCell::new(None),
            should_close: RefCell::new(None),
            accessibility: RefCell::new(None),
            nodes: RefCell::new(BTreeMap::new()),
            a11y_root: Cell::new(None),
            a11y_active: Cell::new(false),
        });
        WINDOWS.with(|windows| windows.borrow_mut().insert(id, state.clone()));
        unsafe { gpui_ios_host_show(view.as_ptr()) };
        Ok(Box::new(IosWindow(state)))
    }
    fn window_appearance(&self) -> WindowAppearance {
        WINDOWS.with(|w| {
            w.borrow()
                .values()
                .next()
                .map(|w| w.appearance.get())
                .unwrap_or(WindowAppearance::Light)
        })
    }
    fn open_url(&self, url: &str) {
        if let Ok(url) = CString::new(url) {
            unsafe { gpui_ios_host_open_url(url.as_ptr()) };
        }
    }
    fn on_open_urls(&self, callback: Box<dyn FnMut(Vec<String>)>) {
        *self.urls.borrow_mut() = Some(callback);
    }
    fn register_url_scheme(&self, _: &str) -> Task<Result<()>> {
        Task::ready(Err(anyhow!("Declare URL schemes in Info.plist")))
    }
    fn prompt_for_paths(
        &self,
        _: PathPromptOptions,
    ) -> oneshot::Receiver<Result<Option<Vec<PathBuf>>>> {
        unsupported_prompt("document picker")
    }
    fn prompt_for_new_path(
        &self,
        _: &Path,
        _: Option<&str>,
    ) -> oneshot::Receiver<Result<Option<PathBuf>>> {
        unsupported_prompt("save document picker")
    }
    fn can_select_mixed_files_and_dirs(&self) -> bool {
        false
    }
    fn reveal_path(&self, _: &Path) {}
    fn open_with_system(&self, _: &Path) {}
    fn on_quit(&self, callback: Box<dyn FnMut() -> bool>) {
        *self.quit.borrow_mut() = Some(callback);
    }
    fn on_reopen(&self, _: Box<dyn FnMut()>) {}
    fn on_system_sleep(&self, _: Box<dyn FnMut()>) {}
    fn on_system_wake(&self, _: Box<dyn FnMut()>) {}
    fn on_app_lifecycle(&self, callback: Box<dyn FnMut(AppLifecyclePhase)>) {
        *self.lifecycle.borrow_mut() = Some(callback);
    }
    fn on_memory_warning(&self, callback: Box<dyn FnMut()>) {
        *self.memory_warning.borrow_mut() = Some(callback);
    }
    fn set_menus(&self, _: Vec<Menu>, _: &Keymap) {}
    fn set_dock_menu(&self, _: Vec<MenuItem>, _: &Keymap) {}
    fn on_app_menu_action(&self, _: Box<dyn FnMut(&dyn Action)>) {}
    fn on_will_open_app_menu(&self, _: Box<dyn FnMut()>) {}
    fn on_validate_app_menu_command(&self, _: Box<dyn FnMut(&dyn Action) -> bool>) {}
    fn thermal_state(&self) -> ThermalState {
        gpui_apple::thermal::thermal_state()
    }
    fn on_thermal_state_change(&self, _: Box<dyn FnMut()>) {}
    fn prevent_idle_sleep(&self, _: &str) -> Task<Result<ActivityGuard>> {
        Task::ready(Err(anyhow!("idle sleep prevention is not implemented")))
    }
    fn app_path(&self) -> Result<PathBuf> {
        Ok(std::env::current_exe()?)
    }
    fn path_for_auxiliary_executable(&self, _: &str) -> Result<PathBuf> {
        bail!("iOS cannot launch auxiliary executables")
    }
    fn set_cursor_style(&self, _: CursorStyle) {}
    fn hide_cursor_until_mouse_moves(&self) {}
    fn is_cursor_visible(&self) -> bool {
        false
    }
    fn should_auto_hide_scrollbars(&self) -> bool {
        true
    }
    fn read_from_clipboard(&self) -> Option<ClipboardItem> {
        let text = crate::bridge::take_host_string(unsafe { gpui_ios_host_clipboard_read() });
        text.map(ClipboardItem::new_string)
    }
    fn write_to_clipboard(&self, item: ClipboardItem) {
        if let Some(text) = item.text().and_then(|t| CString::new(t).ok()) {
            unsafe { gpui_ios_host_clipboard_write(text.as_ptr()) };
        }
        *self.clipboard.borrow_mut() = Some(item);
    }
    fn write_credentials(&self, url: &str, username: &str, password: &[u8]) -> Task<Result<()>> {
        Task::ready(
            gpui_apple::keychain::write_credentials(url, username, password).map_err(Into::into),
        )
    }
    fn read_credentials(&self, url: &str) -> Task<Result<Option<(String, Vec<u8>)>>> {
        Task::ready(gpui_apple::keychain::read_credentials(url).map_err(Into::into))
    }
    fn delete_credentials(&self, url: &str) -> Task<Result<()>> {
        Task::ready(gpui_apple::keychain::delete_credentials(url).map_err(Into::into))
    }
    fn keyboard_layout(&self) -> Box<dyn PlatformKeyboardLayout> {
        Box::new(IosKeyboard)
    }
    fn keyboard_mapper(&self) -> Rc<dyn PlatformKeyboardMapper> {
        Rc::new(DummyKeyboardMapper)
    }
    fn on_keyboard_layout_change(&self, _: Box<dyn FnMut()>) {}
}

fn unsupported_prompt<T>(name: &str) -> oneshot::Receiver<Result<T>> {
    let (tx, rx) = oneshot::channel();
    if tx
        .send(Err(anyhow!("{name} is not implemented by gpui_ios")))
        .is_err()
    {
        log::debug!("prompt receiver dropped");
    }
    rx
}

pub(crate) struct WindowState {
    pub id: u64,
    pub handle: AnyWindowHandle,
    pub view: NonNull<c_void>,
    pub renderer: RefCell<MetalRenderer>,
    pub bounds: Cell<Bounds<Pixels>>,
    pub scale: Cell<f32>,
    pub active: Cell<bool>,
    pub visible: Cell<bool>,
    pub appearance: Cell<WindowAppearance>,
    pub background: Cell<WindowBackgroundAppearance>,
    pub title: RefCell<String>,
    pub insets: RefCell<WindowInsets>,
    pub input_handler: RefCell<Option<PlatformInputHandler>>,
    pub frame: Callback<Box<dyn FnMut(RequestFrameOptions)>>,
    pub input: Callback<Box<dyn FnMut(PlatformInput) -> DispatchEventResult>>,
    pub active_changed: Callback<Box<dyn FnMut(bool)>>,
    pub visibility_changed: Callback<Box<dyn FnMut(WindowVisibility)>>,
    pub resized: Callback<Box<dyn FnMut(Size<Pixels>, f32)>>,
    pub appearance_changed: Callback<Box<dyn FnMut()>>,
    pub inset_changed: Callback<Box<dyn FnMut(WindowInsets)>>,
    pub viewport_changed: Callback<Box<dyn FnMut()>>,
    pub close: Callback<Box<dyn FnOnce()>>,
    pub should_close: Callback<Box<dyn FnMut() -> bool>>,
    pub accessibility: RefCell<Option<A11yCallbacks>>,
    pub nodes: RefCell<BTreeMap<u64, accesskit::Node>>,
    pub a11y_root: Cell<Option<u64>>,
    pub a11y_active: Cell<bool>,
}

pub(crate) fn window(id: u64) -> Option<Rc<WindowState>> {
    WINDOWS.with(|windows| windows.borrow().get(&id).cloned())
}

pub(crate) fn with_input<T>(id: u64, f: impl FnOnce(&mut PlatformInputHandler) -> T) -> Option<T> {
    let state = window(id)?;
    let mut handler = state.input_handler.borrow_mut().take()?;
    let value = f(&mut handler);
    // Input handling may repaint and install a newer handler while the old one is out.
    let mut slot = state.input_handler.borrow_mut();
    if slot.is_none() {
        *slot = Some(handler);
    }
    Some(value)
}

struct IosWindow(Rc<WindowState>);
impl Drop for IosWindow {
    fn drop(&mut self) {
        WINDOWS.with(|windows| windows.borrow_mut().remove(&self.0.id));
        unsafe { gpui_ios_host_destroy(self.0.view.as_ptr()) };
    }
}
impl HasWindowHandle for IosWindow {
    fn window_handle(&self) -> std::result::Result<WindowHandle<'_>, HandleError> {
        // SAFETY: the native host retains the view for this window's lifetime.
        Ok(unsafe { WindowHandle::borrow_raw(UiKitWindowHandle::new(self.0.view).into()) })
    }
}
impl HasDisplayHandle for IosWindow {
    fn display_handle(&self) -> std::result::Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::uikit())
    }
}

impl PlatformWindow for IosWindow {
    fn bounds(&self) -> Bounds<Pixels> {
        self.0.bounds.get()
    }
    fn is_maximized(&self) -> bool {
        false
    }
    fn window_bounds(&self) -> WindowBounds {
        WindowBounds::Fullscreen(self.bounds())
    }
    fn content_size(&self) -> Size<Pixels> {
        self.bounds().size
    }
    fn resize(&mut self, _: Size<Pixels>) {}
    fn scale_factor(&self) -> f32 {
        self.0.scale.get()
    }
    fn appearance(&self) -> WindowAppearance {
        self.0.appearance.get()
    }
    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        Some(Rc::new(IosDisplay))
    }
    fn mouse_position(&self) -> Point<Pixels> {
        Point::default()
    }
    fn modifiers(&self) -> Modifiers {
        Modifiers::default()
    }
    fn capslock(&self) -> Capslock {
        Capslock::default()
    }
    fn set_input_handler(&mut self, handler: PlatformInputHandler) {
        *self.0.input_handler.borrow_mut() = Some(handler);
    }
    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.0.input_handler.borrow_mut().take()
    }
    fn set_text_input_configuration(&mut self, config: TextInputConfiguration) {
        let capitalize = match config.autocapitalize {
            Autocapitalize::None => 0,
            Autocapitalize::Words => 1,
            Autocapitalize::Sentences => 2,
            Autocapitalize::Characters => 3,
        };
        unsafe {
            gpui_ios_host_text_configuration(
                self.0.view.as_ptr(),
                config.autocorrect,
                capitalize,
                config.suggestions,
            )
        };
    }
    fn prompt(
        &self,
        _: PromptLevel,
        _: &str,
        _: Option<&str>,
        _: &[PromptButton],
    ) -> Option<oneshot::Receiver<usize>> {
        None
    }
    fn activate(&self) {}
    fn is_active(&self) -> bool {
        self.0.active.get()
    }
    fn visibility(&self) -> WindowVisibility {
        if self.0.visible.get() {
            WindowVisibility::Visible
        } else {
            WindowVisibility::Hidden
        }
    }
    fn is_hovered(&self) -> bool {
        false
    }
    fn background_appearance(&self) -> WindowBackgroundAppearance {
        self.0.background.get()
    }
    fn set_title(&mut self, title: &str) {
        *self.0.title.borrow_mut() = title.to_owned();
    }
    fn set_background_appearance(&self, value: WindowBackgroundAppearance) {
        self.0.background.set(value);
    }
    fn minimize(&self) {}
    fn zoom(&self) {}
    fn toggle_fullscreen(&self) {}
    fn is_fullscreen(&self) -> bool {
        true
    }
    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>) {
        *self.0.frame.borrow_mut() = Some(callback);
    }
    fn on_input(&self, callback: Box<dyn FnMut(PlatformInput) -> DispatchEventResult>) {
        *self.0.input.borrow_mut() = Some(callback);
    }
    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        *self.0.active_changed.borrow_mut() = Some(callback);
    }
    fn on_visibility_change(&self, callback: Box<dyn FnMut(WindowVisibility)>) {
        *self.0.visibility_changed.borrow_mut() = Some(callback);
    }
    fn on_hover_status_change(&self, _: Box<dyn FnMut(bool)>) {}
    fn on_resize(&self, callback: Box<dyn FnMut(Size<Pixels>, f32)>) {
        *self.0.resized.borrow_mut() = Some(callback);
    }
    fn on_moved(&self, _: Box<dyn FnMut()>) {}
    fn on_display_changed(&self, _: Box<dyn FnMut()>) {}
    fn on_should_close(&self, callback: Box<dyn FnMut() -> bool>) {
        *self.0.should_close.borrow_mut() = Some(callback);
    }
    fn on_hit_test_window_control(&self, _: Box<dyn FnMut() -> Option<WindowControlArea>>) {}
    fn on_close(&self, callback: Box<dyn FnOnce()>) {
        *self.0.close.borrow_mut() = Some(callback);
    }
    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>) {
        *self.0.appearance_changed.borrow_mut() = Some(callback);
    }
    fn draw(&self, scene: &Scene) {
        if self.0.visible.get() {
            self.0.renderer.borrow_mut().draw(scene);
            // The Swift host defers this query until GPUI has released its App borrow.
            // It compares snapshots before notifying UIKit, including programmatic edits.
            unsafe { gpui_ios_host_text_changed(self.0.view.as_ptr()) };
        }
    }
    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.0.renderer.borrow().sprite_atlas().clone()
    }
    fn is_subpixel_rendering_supported(&self) -> bool {
        false
    }
    fn gpu_specs(&self) -> Option<GpuSpecs> {
        None
    }
    fn update_ime_position(&self, _: Bounds<Pixels>) {}
    fn insets(&self) -> WindowInsets {
        self.0.insets.borrow().clone()
    }
    fn on_insets_changed(&self, callback: Box<dyn FnMut(WindowInsets)>) {
        *self.0.inset_changed.borrow_mut() = Some(callback);
    }
    fn on_visual_viewport_changed(&self, callback: Box<dyn FnMut()>) {
        *self.0.viewport_changed.borrow_mut() = Some(callback);
    }
    fn show_soft_keyboard(&self) {
        unsafe { gpui_ios_host_keyboard(self.0.view.as_ptr(), true) };
    }
    fn hide_soft_keyboard(&self) {
        unsafe { gpui_ios_host_keyboard(self.0.view.as_ptr(), false) };
    }
    fn text_input_state_changed(&self, change: TextInputStateChange) {
        match change {
            TextInputStateChange::FocusGained => self.show_soft_keyboard(),
            TextInputStateChange::FocusLost => self.hide_soft_keyboard(),
            _ => {}
        }
        unsafe { gpui_ios_host_text_changed(self.0.view.as_ptr()) };
    }
    fn a11y_init(&self, callbacks: A11yCallbacks) {
        *self.0.accessibility.borrow_mut() = Some(callbacks);
    }
    fn a11y_tree_update(&self, update: accesskit::TreeUpdate) {
        crate::bridge::accessibility_update(&self.0, update);
    }
}

unsafe extern "C" {
    pub(crate) fn gpui_ios_host_create(id: u64) -> *mut c_void;
    pub(crate) fn gpui_ios_host_layer(view: *mut c_void) -> *mut c_void;
    pub(crate) fn gpui_ios_host_show(view: *mut c_void);
    pub(crate) fn gpui_ios_host_destroy(view: *mut c_void);
    pub(crate) fn gpui_ios_host_screen(width: *mut f32, height: *mut f32, scale: *mut f32);
    pub(crate) fn gpui_ios_host_keyboard(view: *mut c_void, visible: bool);
    pub(crate) fn gpui_ios_host_text_changed(view: *mut c_void);
    pub(crate) fn gpui_ios_host_text_configuration(
        view: *mut c_void,
        autocorrect: bool,
        capitalization: i32,
        suggestions: bool,
    );
    pub(crate) fn gpui_ios_host_open_url(url: *const i8);
    pub(crate) fn gpui_ios_host_clipboard_read() -> *mut i8;
    pub(crate) fn gpui_ios_host_clipboard_write(text: *const i8);
    pub(crate) fn gpui_ios_host_free_string(text: *mut i8);
    pub(crate) fn gpui_ios_host_data_directory() -> *mut i8;
    pub(crate) fn gpui_ios_host_accessibility(view: *mut c_void, json: *const i8);
}
