//! Native GTK3 overlay renderer.
//!
//! The overlay used to be a transparent WebKit web view. WebKitGTK keeps a web
//! process (~95-155 MiB PSS) and a network process (~30 MiB) alive for every
//! web view, which put a hard floor of roughly 265 MiB on the whole application
//! even when idle. The overlay is a small, mostly static list, so it is drawn
//! directly with GTK/cairo instead: no web process, no network process, no
//! JavaScript heap, and no continuous compositor work while nothing changes.
//!
//! The settings panel deliberately stays a Tauri/WebKit window. It is created on
//! first use and destroyed when closed, so the idle overlay never pays for it.
//!
//! Everything here runs on the GTK main thread. Public functions marshal calls
//! from Tauri command threads and the Discord worker onto the main loop.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::f64::consts::PI;
use std::io::Read;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread::ThreadId;
use std::time::Duration;

use discord_overlay_core::client::Status;
use discord_overlay_core::config::{Background, Config, Layout, PlacementMode};
use discord_overlay_core::geometry::{
    clamp_into_monitor, compute_placement, is_offscreen, MonitorRect, WindowPlacement,
};
use discord_overlay_core::state::{OverlayState, Participant};
use gtk::prelude::*;
use gtk::{cairo, gdk, gdk_pixbuf, gio, glib, pango};
use tauri::{AppHandle, Manager};

use crate::state::{lock, AppState};
use crate::windows::MonitorInfo;

const MAX_AVATARS: usize = 48;
const MAX_INFLIGHT_AVATARS: usize = 3;
const AVATAR_BYTES_LIMIT: u64 = 2 * 1024 * 1024;

thread_local! {
    static OVERLAY: RefCell<Option<Rc<NativeOverlay>>> = const { RefCell::new(None) };
    static MAIN_THREAD: Cell<Option<ThreadId>> = const { Cell::new(None) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    ToggleClickThrough,
    ToggleLock,
    ResetPosition,
    OpenSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Icon {
    Grip,
    Lock,
    Unlock,
    Pointer,
    PointerOff,
    Refresh,
    Settings,
    Mic,
    MicOff,
    HeadphonesOff,
    Bot,
}

struct Hit {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    action: Action,
    tooltip: &'static str,
}

impl Hit {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }
}

struct View {
    config: Config,
    overlay: OverlayState,
    status: Status,
    hits: Vec<Hit>,
    last_click_through: Option<bool>,
}

struct AvatarMsg {
    url: String,
    result: Result<Vec<u8>, String>,
}

struct AvatarCache {
    images: HashMap<String, gdk_pixbuf::Pixbuf>,
    order: VecDeque<String>,
    inflight: HashSet<String>,
    failed: HashSet<String>,
    sender: glib::Sender<AvatarMsg>,
}

struct NativeOverlay {
    window: gtk::Window,
    area: gtk::DrawingArea,
    view: Rc<RefCell<View>>,
    avatars: Rc<RefCell<AvatarCache>>,
    placement_key: Rc<RefCell<String>>,
    app: AppHandle,
}

fn on_main_thread() -> bool {
    MAIN_THREAD.with(|cell| cell.get() == Some(std::thread::current().id()))
}

fn with_overlay<R>(f: impl FnOnce(&NativeOverlay) -> R) -> Option<R> {
    OVERLAY.with(|slot| slot.borrow().as_ref().map(|overlay| f(overlay)))
}

fn schedule<F>(app: &AppHandle, f: F)
where
    F: FnOnce(&NativeOverlay) + Send + 'static,
{
    if on_main_thread() {
        let _ = with_overlay(f);
    } else {
        let app = app.clone();
        let _ = app.run_on_main_thread(move || {
            let _ = with_overlay(f);
        });
    }
}

fn query<R, F>(app: &AppHandle, f: F) -> Option<R>
where
    R: Send + 'static,
    F: FnOnce(&NativeOverlay) -> R + Send + 'static,
{
    if on_main_thread() {
        with_overlay(f)
    } else {
        let (tx, rx) = mpsc::channel();
        app.clone()
            .run_on_main_thread(move || {
                let value = with_overlay(f);
                let _ = tx.send(value);
            })
            .ok()?;
        rx.recv_timeout(Duration::from_secs(2)).ok().flatten()
    }
}

/// Build the native overlay window. Must be called from Tauri's setup hook,
/// which runs on the GTK main thread.
pub fn init(
    app: &AppHandle,
    config: &Config,
    overlay: &OverlayState,
    status: &Status,
) -> Result<(), String> {
    if !gtk::is_initialized() {
        gtk::init().map_err(|error| format!("could not initialise GTK: {error}"))?;
    }
    MAIN_THREAD.with(|cell| cell.set(Some(std::thread::current().id())));

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Discord Linux Overlay");
    window.set_decorated(false);
    window.set_resizable(!config.behavior.lock_position);
    window.set_keep_above(config.behavior.always_on_top);
    window.set_skip_taskbar_hint(true);
    window.set_skip_pager_hint(true);
    window.set_accept_focus(false);
    window.set_app_paintable(true);
    window.set_default_size(
        config.placement.width.round() as i32,
        config.placement.height.round() as i32,
    );
    if let Some(screen) = gtk::prelude::WidgetExt::screen(&window) {
        if let Some(visual) = screen.rgba_visual() {
            window.set_visual(Some(&visual));
        }
    }

    let area = gtk::DrawingArea::new();
    area.set_has_tooltip(true);
    window.add(&area);

    let view = Rc::new(RefCell::new(View {
        config: config.clone(),
        overlay: overlay.clone(),
        status: status.clone(),
        hits: Vec::new(),
        last_click_through: None,
    }));
    let (cache, receiver) = AvatarCache::new();
    let avatars = Rc::new(RefCell::new(cache));
    let placement_key = Rc::new(RefCell::new(placement_key(config)));

    // Drawing.
    {
        let view = view.clone();
        let avatars = avatars.clone();
        area.connect_draw(move |area, cr| {
            render(area, cr, &mut view.borrow_mut(), &avatars.borrow());
            glib::Propagation::Proceed
        });
    }

    // Clicking: toolbar buttons, otherwise start a compositor move when the
    // position is unlocked. Click-through windows never see these events.
    {
        let app = app.clone();
        let view = view.clone();
        area.connect_button_press_event(move |area, event| {
            let (x, y) = event.position();
            let action = {
                let view = view.borrow();
                view.hits
                    .iter()
                    .find(|hit| hit.contains(x, y))
                    .map(|hit| hit.action)
            };

            if let Some(action) = action {
                if event.button() == 1 {
                    dispatch(&app, action);
                }
                return glib::Propagation::Stop;
            }

            let locked = view.borrow().config.behavior.lock_position;
            if !locked && event.button() == 1 {
                if let Some(window) = area.toplevel().and_downcast::<gtk::Window>() {
                    let (root_x, root_y) = event.root();
                    window.begin_move_drag(
                        event.button() as i32,
                        root_x as i32,
                        root_y as i32,
                        event.time(),
                    );
                    return glib::Propagation::Stop;
                }
            }
            glib::Propagation::Proceed
        });
    }

    // Tooltips for the toolbar buttons.
    {
        let view = view.clone();
        area.connect_query_tooltip(move |_, x, y, _, tooltip| {
            let view = view.borrow();
            if let Some(hit) = view
                .hits
                .iter()
                .find(|hit| hit.contains(f64::from(x), f64::from(y)))
            {
                tooltip.set_text(Some(hit.tooltip));
                return true;
            }
            false
        });
    }

    // Remember manually dragged/moved bounds, debounced.
    {
        let app = app.clone();
        let view = view.clone();
        let placement_key = placement_key.clone();
        window.connect_configure_event(move |window, _| {
            let state = app.state::<AppState>();
            let mut config = state.config();
            if config.placement.mode != PlacementMode::Manual || !window.is_visible() {
                return false;
            }
            let mut last_save = lock(&state.last_bounds_save);
            if last_save.elapsed() < Duration::from_millis(500) {
                return false;
            }
            *last_save = std::time::Instant::now();
            drop(last_save);

            let (width, height) = window.size();
            config.placement.width = width.max(64) as f64;
            config.placement.height = height.max(64) as f64;
            if !crate::platform::is_wayland() {
                let (x, y) = window.position();
                config.placement.x = Some(x);
                config.placement.y = Some(y);
            }

            if let Err(error) = state.config_store.save(&config) {
                log::warn!("could not persist overlay bounds: {error}");
                return false;
            }
            *lock(&state.config) = config.clone();
            *placement_key.borrow_mut() = placement_key_for(&config);
            let _ = &view;
            false
        });
    }

    // Avatar downloads decode and land back on the main loop.
    {
        let avatars = avatars.clone();
        let view = view.clone();
        let area = area.clone();
        receiver.attach(Some(&glib::MainContext::default()), move |message| {
            {
                let mut cache = avatars.borrow_mut();
                cache.inflight.remove(&message.url);
                match message.result {
                    Ok(bytes) => match decode_avatar(&bytes) {
                        Ok(pixbuf) => cache.insert(message.url, pixbuf),
                        Err(error) => {
                            log::debug!("could not decode avatar: {error}");
                            cache.failed.insert(message.url);
                        }
                    },
                    Err(error) => {
                        log::debug!("could not download avatar: {error}");
                        cache.failed.insert(message.url);
                    }
                }
            }
            request_avatars(&view, &mut avatars.borrow_mut());
            area.queue_draw();
            glib::ControlFlow::Continue
        });
    }

    let native = Rc::new(NativeOverlay {
        window: window.clone(),
        area: area.clone(),
        view: view.clone(),
        avatars: avatars.clone(),
        placement_key,
        app: app.clone(),
    });

    // GTK3 has no xdg-decoration support, so KWin falls back to server-side
    // decorations for undecorated windows. Giving the window an empty CSD
    // titlebar (the same trick tao uses for Tauri windows) suppresses it and
    // leaves a genuinely borderless, transparent overlay on Wayland.
    if gtk::gdk::Display::default()
        .map(|display| display.backend().is_wayland())
        .unwrap_or(false)
    {
        let titlebar = gtk::EventBox::new();
        window.set_titlebar(Some(&titlebar));
    }
    window.show_all();
    window.set_decorated(false);
    window.resize(
        config.placement.width.round() as i32,
        config.placement.height.round() as i32,
    );

    OVERLAY.with(|slot| *slot.borrow_mut() = Some(native));
    apply_placement_now(app, config)?;
    refresh_now(app);
    Ok(())
}

/// Re-read application state and repaint. Cheap; called for every event.
pub fn refresh(app: &AppHandle) {
    if on_main_thread() {
        refresh_now(app);
    } else {
        let app = app.clone();
        let worker_app = app.clone();
        let _ = app.run_on_main_thread(move || refresh_now(&worker_app));
    }
}

fn refresh_now(app: &AppHandle) {
    let Some(native) = with_overlay(|overlay| overlay as *const NativeOverlay) else {
        return;
    };
    // SAFETY: the pointer is only used on the GTK main thread while the
    // thread-local overlay is alive; no reference escapes this function.
    let native = unsafe { &*native };
    let state = app.state::<AppState>();
    let config = state.config();
    let overlay_state = lock(&state.overlay).clone();
    let status = lock(&state.status).clone();

    native.window.set_keep_above(config.behavior.always_on_top);
    native.window.set_resizable(!config.behavior.lock_position);
    native.window.set_skip_taskbar_hint(true);
    native.window.set_skip_pager_hint(true);

    {
        let mut view = native.view.borrow_mut();
        view.config = config.clone();
        view.overlay = overlay_state;
        view.status = status;
    }
    set_click_through(native, config.behavior.click_through);

    let key = placement_key(&config);
    if *native.placement_key.borrow() != key {
        *native.placement_key.borrow_mut() = key;
        if let Err(error) = apply_placement_now(app, &config) {
            log::warn!("could not apply overlay placement: {error}");
        }
    }

    request_avatars(&native.view, &mut native.avatars.borrow_mut());
    native.area.queue_draw();
}

fn set_click_through(native: &NativeOverlay, click_through: bool) {
    let changed = {
        let mut view = native.view.borrow_mut();
        let changed = view.last_click_through != Some(click_through);
        view.last_click_through = Some(click_through);
        changed
    };
    if !changed {
        return;
    }
    if click_through {
        let region = cairo::Region::create();
        native.window.input_shape_combine_region(Some(&region));
    } else {
        native.window.input_shape_combine_region(None);
    }
}

pub fn show(app: &AppHandle) {
    let config = app.state::<AppState>().config();
    schedule(app, move |native| {
        native.window.show_all();
        native.window.set_keep_above(config.behavior.always_on_top);
        if let Err(error) = apply_placement_now(&native.app, &config) {
            log::debug!("could not place the overlay: {error}");
        }
        native.area.queue_draw();
    });
}

pub fn hide(app: &AppHandle) {
    schedule(app, |native| {
        native.window.hide();
    });
}

pub fn is_visible(app: &AppHandle) -> bool {
    query(app, |native| native.window.is_visible()).unwrap_or(false)
}

pub fn apply_behavior(app: &AppHandle, config: &Config) {
    let config = config.clone();
    schedule(app, move |native| {
        native.window.set_keep_above(config.behavior.always_on_top);
        native.window.set_resizable(!config.behavior.lock_position);
        set_click_through(native, config.behavior.click_through);
        native.area.queue_draw();
    });
}

pub fn apply_placement(app: &AppHandle, config: &Config) -> Result<(), String> {
    let config = config.clone();
    let app = app.clone();
    if on_main_thread() {
        return with_overlay(|native| apply_placement_now(&native.app, &config)).unwrap_or(Ok(()));
    }
    let (tx, rx) = mpsc::channel();
    app.clone()
        .run_on_main_thread(move || {
            let result =
                with_overlay(|native| apply_placement_now(&native.app, &config)).unwrap_or(Ok(()));
            let _ = tx.send(result);
        })
        .map_err(|error| error.to_string())?;
    rx.recv_timeout(Duration::from_secs(2))
        .map_err(|_| "timed out while placing the overlay".to_string())?
}

fn apply_placement_now(app: &AppHandle, config: &Config) -> Result<(), String> {
    let Some(native) = with_overlay(|overlay| overlay as *const NativeOverlay) else {
        return Ok(());
    };
    let native = unsafe { &*native };
    let window = &native.window;
    let wayland = crate::platform::is_wayland();
    let (logical_width, logical_height) = (
        config.placement.width.clamp(64.0, 10000.0),
        config.placement.height.clamp(64.0, 10000.0),
    );

    if config.placement.mode == PlacementMode::Manual {
        let rects = monitor_rects();
        let mut placement = WindowPlacement {
            x: config.placement.x.unwrap_or(0),
            y: config.placement.y.unwrap_or(0),
            width: logical_width.round() as u32,
            height: logical_height.round() as u32,
        };
        if !rects.is_empty() && is_offscreen(placement, &rects) {
            if let Some(primary) = rects.first().copied() {
                placement = clamp_into_monitor(placement, primary);
            }
        }
        if !wayland {
            window.move_(placement.x, placement.y);
        }
        window.resize(placement.width as i32, placement.height as i32);
        return Ok(());
    }

    let monitors = monitor_infos();
    let monitor = match config.placement.monitor.as_deref() {
        Some(name) => monitors
            .iter()
            .find(|monitor| monitor.name == name)
            .or_else(|| monitors.first()),
        None => monitors
            .iter()
            .find(|monitor| monitor.primary)
            .or_else(|| monitors.first()),
    };
    let Some(monitor) = monitor else {
        window.resize(logical_width.round() as i32, logical_height.round() as i32);
        return Ok(());
    };

    let scale = if monitor.scale_factor > 0.0 {
        monitor.scale_factor
    } else {
        1.0
    };
    let rect = MonitorRect::new(monitor.x, monitor.y, monitor.width, monitor.height);
    let placement = compute_placement(
        rect,
        (logical_width, logical_height),
        scale,
        config.placement.anchor,
        (config.placement.offset_x, config.placement.offset_y),
    )
    .map_err(|error| error.to_string())?;

    if !wayland {
        window.move_(placement.x, placement.y);
    }
    window.resize(
        ((placement.width as f64 / scale).round() as i32).max(64),
        ((placement.height as f64 / scale).round() as i32).max(64),
    );
    let _ = app;
    Ok(())
}

/// Lines for `--diagnose`.
pub fn describe(app: &AppHandle) -> Vec<String> {
    query(app, |native| {
        let mut lines = Vec::new();
        lines.push(format!(
            "visible: {}",
            if native.window.is_visible() {
                "yes"
            } else {
                "no"
            }
        ));
        let (width, height) = native.window.size();
        lines.push(format!(
            "size: {width}x{height} (logical pixels, native GTK)",
            width = width,
            height = height
        ));
        let (x, y) = native.window.position();
        if crate::platform::is_wayland() {
            lines.push(format!(
                "position: compositor-managed (reported {x},{y}); Wayland ignores app moves"
            ));
        } else {
            lines.push(format!("position: {x},{y}"));
        }
        lines.push(
            "renderer: native GTK3/cairo (no web view, no WebKit child processes)".to_string(),
        );
        lines.push(format!(
            "decorated: {}, app-paintable: {}, rgba visual: {}",
            native.window.is_decorated(),
            native.window.is_app_paintable(),
            gtk::prelude::WidgetExt::screen(&native.window)
                .and_then(|screen| screen.rgba_visual())
                .is_some()
        ));
        lines.push(format!(
            "avatar cache: {}/{} entries",
            native.avatars.borrow().images.len(),
            MAX_AVATARS
        ));
        lines
    })
    .unwrap_or_else(|| vec!["window: not created".to_string()])
}

pub fn monitor_list(app: &AppHandle) -> Result<Vec<MonitorInfo>, String> {
    if on_main_thread() {
        return Ok(monitor_infos());
    }
    let (tx, rx) = mpsc::channel();
    app.clone()
        .run_on_main_thread(move || {
            let _ = tx.send(monitor_infos());
        })
        .map_err(|error| error.to_string())?;
    rx.recv_timeout(Duration::from_secs(2))
        .map_err(|_| "timed out while listing monitors".to_string())
}

fn monitor_infos() -> Vec<MonitorInfo> {
    let Some(display) = gdk::Display::default() else {
        return Vec::new();
    };
    let mut monitors = Vec::new();
    for index in 0..display.n_monitors() {
        let Some(monitor) = display.monitor(index) else {
            continue;
        };
        let geometry = monitor.geometry();
        let scale = f64::from(monitor.scale_factor().max(1));
        monitors.push(MonitorInfo {
            name: monitor
                .model()
                .map(|model| model.to_string())
                .unwrap_or_else(|| format!("monitor-{index}")),
            x: (f64::from(geometry.x()) * scale).round() as i32,
            y: (f64::from(geometry.y()) * scale).round() as i32,
            width: (f64::from(geometry.width()) * scale).round() as u32,
            height: (f64::from(geometry.height()) * scale).round() as u32,
            scale_factor: scale,
            primary: monitor.is_primary(),
        });
    }
    monitors
}

fn monitor_rects() -> Vec<MonitorRect> {
    monitor_infos()
        .into_iter()
        .map(|monitor| MonitorRect::new(monitor.x, monitor.y, monitor.width, monitor.height))
        .collect()
}

fn placement_key(config: &Config) -> String {
    placement_key_for(config)
}

fn placement_key_for(config: &Config) -> String {
    format!(
        "{:?}|{:?}|{:?}|{}|{}|{}|{}|{:?}|{:?}",
        config.placement.mode,
        config.placement.monitor,
        config.placement.anchor,
        config.placement.offset_x,
        config.placement.offset_y,
        config.placement.width,
        config.placement.height,
        config.placement.x,
        config.placement.y
    )
}

fn dispatch(app: &AppHandle, action: Action) {
    let request = match action {
        Action::ToggleClickThrough => discord_overlay_core::control::Request::ToggleClickThrough,
        Action::ToggleLock => discord_overlay_core::control::Request::ToggleLock,
        Action::ResetPosition => discord_overlay_core::control::Request::ResetPosition,
        Action::OpenSettings => discord_overlay_core::control::Request::OpenSettings,
    };
    let _ = crate::commands::apply_request(app, request);
}

// ------------------------------------------------------------------ avatars

impl AvatarCache {
    fn new() -> (Self, glib::Receiver<AvatarMsg>) {
        let (sender, receiver) = glib::MainContext::channel(glib::Priority::default());
        (
            Self {
                images: HashMap::new(),
                order: VecDeque::new(),
                inflight: HashSet::new(),
                failed: HashSet::new(),
                sender,
            },
            receiver,
        )
    }

    fn insert(&mut self, url: String, pixbuf: gdk_pixbuf::Pixbuf) {
        if self.images.insert(url.clone(), pixbuf).is_none() {
            self.order.push_back(url);
        }
        while self.images.len() > MAX_AVATARS {
            if let Some(oldest) = self.order.pop_front() {
                self.images.remove(&oldest);
            } else {
                break;
            }
        }
    }
}

fn valid_avatar_url(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://")) && url.len() < 2048
}

fn request_avatars(view: &Rc<RefCell<View>>, cache: &mut AvatarCache) {
    let urls: Vec<String> = {
        let view = view.borrow();
        view.overlay
            .participants
            .iter()
            .map(|participant| participant.avatar_url.clone())
            .filter(|url| valid_avatar_url(url))
            .collect()
    };

    for url in urls {
        if cache.images.contains_key(&url)
            || cache.inflight.contains(&url)
            || cache.failed.contains(&url)
        {
            continue;
        }
        if cache.inflight.len() >= MAX_INFLIGHT_AVATARS {
            break;
        }
        cache.inflight.insert(url.clone());
        let sender = cache.sender.clone();
        std::thread::spawn(move || {
            let result = fetch_avatar(&url);
            let _ = sender.send(AvatarMsg { url, result });
        });
    }
}

fn fetch_avatar(url: &str) -> Result<Vec<u8>, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(6))
        .build();
    let response = agent
        .get(url)
        .set("User-Agent", "DiscordLinuxOverlay/0.1")
        .call()
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(AVATAR_BYTES_LIMIT)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

fn decode_avatar(bytes: &[u8]) -> Result<gdk_pixbuf::Pixbuf, String> {
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(bytes.to_vec()));
    gdk_pixbuf::Pixbuf::from_stream(&stream, None::<&gio::Cancellable>)
        .map_err(|error| error.to_string())
}

// ------------------------------------------------------------------ drawing

fn render(area: &gtk::DrawingArea, cr: &cairo::Context, view: &mut View, avatars: &AvatarCache) {
    let width = f64::from(area.allocated_width());
    let height = f64::from(area.allocated_height());
    if width <= 0.0 || height <= 0.0 {
        return;
    }

    cr.save();
    cr.set_operator(cairo::Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    cr.paint();
    cr.restore();

    let appearance = view.config.appearance.clone();
    let scale = f64::from(appearance.scale) / 100.0;
    let opacity = f64::from(appearance.opacity) / 100.0;
    let radius = f64::from(appearance.corner_radius) * scale;
    let gap = f64::from(appearance.gap) * scale;
    let avatar = f64::from(appearance.avatar_size) * scale;
    let font = 13.0 * scale;
    let padding = 6.0;
    let click_through = view.config.behavior.click_through;
    let align_end = matches!(
        view.config.placement.anchor,
        discord_overlay_core::geometry::Anchor::BottomLeft
            | discord_overlay_core::geometry::Anchor::BottomCenter
            | discord_overlay_core::geometry::Anchor::BottomRight
            | discord_overlay_core::geometry::Anchor::MiddleLeft
            | discord_overlay_core::geometry::Anchor::Center
            | discord_overlay_core::geometry::Anchor::MiddleRight
    );

    view.hits.clear();
    let mut content_top = padding;
    if !click_through {
        content_top += draw_toolbar(cr, view, width, padding, radius, gap, opacity);
    }

    let participants: Vec<&Participant> = view
        .overlay
        .participants
        .iter()
        .filter(|participant| appearance.show_own_user || !participant.is_self)
        .filter(|participant| !appearance.show_only_speaking || participant.speaking)
        .collect();

    let content_bottom = height - padding;
    let content_height = (content_bottom - content_top).max(0.0);

    if appearance.layout == Layout::Stack {
        draw_stack(
            cr,
            view,
            avatars,
            &participants,
            width,
            content_top,
            content_bottom,
            content_height,
            avatar,
            radius,
            gap,
            font,
            scale,
            opacity,
            align_end,
        );
    } else {
        draw_strip(
            cr,
            view,
            avatars,
            &participants,
            width,
            content_top,
            content_bottom,
            avatar,
            radius,
            gap,
            font,
            scale,
            opacity,
            appearance.layout == Layout::Grid,
            align_end,
        );
    }

    if participants.is_empty() && view.config.general.show_connection_status {
        draw_empty_state(
            cr,
            view,
            width,
            content_top,
            content_bottom,
            radius,
            font,
            scale,
            opacity,
            align_end,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_toolbar(
    cr: &cairo::Context,
    view: &mut View,
    width: f64,
    padding: f64,
    radius: f64,
    gap: f64,
    opacity: f64,
) -> f64 {
    let height = 26.0;
    let x = padding;
    let y = padding;
    let w = width - padding * 2.0;

    round_rect(cr, x, y, w, height, radius * 0.75);
    cr.set_source_rgba(17.0 / 255.0, 18.0 / 255.0, 20.0 / 255.0, 0.72 * opacity);
    cr.fill();
    round_rect(cr, x, y, w, height, radius * 0.75);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.06 * opacity);
    cr.set_line_width(1.0);
    cr.stroke();

    let locked = view.config.behavior.lock_position;
    let grip_color = if locked {
        (240.0 / 255.0, 178.0 / 255.0, 50.0 / 255.0, opacity)
    } else {
        (109.0 / 255.0, 111.0 / 255.0, 120.0 / 255.0, opacity)
    };
    draw_icon(
        cr,
        if locked { Icon::Lock } else { Icon::Grip },
        x + 11.0,
        y + height / 2.0,
        12.0,
        grip_color,
    );

    let buttons = [
        (
            Action::ToggleClickThrough,
            Icon::Pointer,
            "Enable click-through",
        ),
        (
            Action::ToggleLock,
            if locked { Icon::Unlock } else { Icon::Lock },
            "Lock or unlock the overlay position",
        ),
        (Action::ResetPosition, Icon::Refresh, "Reset position"),
        (Action::OpenSettings, Icon::Settings, "Settings"),
    ];
    let button_size = 22.0;
    let button_gap = 2.0;
    let total = button_size * buttons.len() as f64 + button_gap * (buttons.len() - 1) as f64;
    let mut button_x = x + w - 6.0 - total;
    let channel_x = x + 20.0;
    let channel_max = (button_x - 6.0 - channel_x).max(10.0);

    let channel = view
        .overlay
        .channel
        .as_ref()
        .map(|channel| format!("#{}", channel.name))
        .unwrap_or_else(|| "Discord voice".to_string());
    draw_text(
        cr,
        &channel,
        channel_x,
        y + height / 2.0,
        11.0,
        (154.0 / 255.0, 156.0 / 255.0, 165.0 / 255.0, opacity),
        channel_max,
        None,
    );

    for (action, icon, tooltip) in buttons {
        let button_y = y + (height - button_size) / 2.0;
        view.hits.push(Hit {
            x: button_x,
            y: button_y,
            width: button_size,
            height: button_size,
            action,
            tooltip,
        });
        draw_icon(
            cr,
            icon,
            button_x + button_size / 2.0,
            button_y + button_size / 2.0,
            15.0,
            (183.0 / 255.0, 185.0 / 255.0, 194.0 / 255.0, opacity),
        );
        button_x += button_size + button_gap;
    }

    height + gap
}

#[allow(clippy::too_many_arguments)]
fn draw_stack(
    cr: &cairo::Context,
    view: &View,
    avatars: &AvatarCache,
    participants: &[&Participant],
    width: f64,
    top: f64,
    bottom: f64,
    content_height: f64,
    avatar: f64,
    radius: f64,
    gap: f64,
    font: f64,
    scale: f64,
    opacity: f64,
    align_end: bool,
) {
    if participants.is_empty() {
        return;
    }
    let row_height = avatar + 10.0;
    let rows_height = participants.len() as f64 * row_height
        + (participants.len().saturating_sub(1)) as f64 * gap;
    let mut y = if align_end {
        (bottom - rows_height).max(top)
    } else {
        top
    };

    let row_width = width - 12.0;
    for participant in participants {
        if y + row_height > bottom + 0.5 {
            break;
        }
        draw_participant_row(
            cr,
            view,
            avatars,
            participant,
            6.0,
            y,
            row_width,
            row_height,
            avatar,
            radius,
            font,
            scale,
            opacity,
            false,
        );
        y += row_height + gap;
    }
    let _ = content_height;
}

#[allow(clippy::too_many_arguments)]
fn draw_strip(
    cr: &cairo::Context,
    view: &View,
    avatars: &AvatarCache,
    participants: &[&Participant],
    width: f64,
    top: f64,
    bottom: f64,
    avatar: f64,
    radius: f64,
    gap: f64,
    font: f64,
    scale: f64,
    opacity: f64,
    grid: bool,
    align_end: bool,
) {
    if participants.is_empty() {
        return;
    }
    let cell = avatar + 8.0;
    let cell_gap = gap;
    let content_width = width - 12.0;
    let per_row = if grid {
        ((content_width + cell_gap) / (cell + cell_gap))
            .floor()
            .max(1.0) as usize
    } else {
        participants.len()
    };
    let rows = participants.len().div_ceil(per_row);
    let total_height = rows as f64 * cell + (rows.saturating_sub(1)) as f64 * cell_gap;
    let mut y = if align_end && grid {
        (bottom - total_height).max(top)
    } else if align_end {
        bottom - cell
    } else {
        top
    };

    let mut index = 0;
    while index < participants.len() {
        let count = per_row.min(participants.len() - index);
        let total_width = count as f64 * cell + (count.saturating_sub(1)) as f64 * cell_gap;
        let mut x = if align_end && !grid {
            (width - 6.0 - total_width).max(6.0)
        } else {
            6.0
        };
        for participant in &participants[index..index + count] {
            if y + cell > bottom + 0.5 {
                return;
            }
            draw_participant_row(
                cr,
                view,
                avatars,
                participant,
                x,
                y,
                cell,
                cell,
                avatar,
                radius,
                font,
                scale,
                opacity,
                true,
            );
            x += cell + cell_gap;
        }
        y += cell + cell_gap;
        index += count;
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_participant_row(
    cr: &cairo::Context,
    view: &View,
    avatars: &AvatarCache,
    participant: &Participant,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    avatar: f64,
    radius: f64,
    font: f64,
    scale: f64,
    opacity: f64,
    compact: bool,
) {
    let appearance = &view.config.appearance;
    let muted = participant.self_muted || participant.muted || participant.suppressed;
    let deafened = participant.self_deafened || participant.deafened;
    let background = appearance.background == Background::Solid;
    let pill_radius = if compact { height / 2.0 } else { radius };

    if background || participant.speaking {
        if participant.speaking {
            round_rect(cr, x, y, width, height, pill_radius);
            cr.set_source_rgba(24.0 / 255.0, 38.0 / 255.0, 30.0 / 255.0, 0.82 * opacity);
            cr.fill();
            round_rect(cr, x + 0.5, y + 0.5, width - 1.0, height - 1.0, pill_radius);
            cr.set_source_rgba(35.0 / 255.0, 165.0 / 255.0, 90.0 / 255.0, 0.9 * opacity);
            cr.set_line_width(1.0);
            cr.stroke();
        } else {
            round_rect(cr, x, y, width, height, pill_radius);
            cr.set_source_rgba(17.0 / 255.0, 18.0 / 255.0, 20.0 / 255.0, 0.72 * opacity);
            cr.fill();
            round_rect(cr, x + 0.5, y + 0.5, width - 1.0, height - 1.0, pill_radius);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.06 * opacity);
            cr.set_line_width(1.0);
            cr.stroke();
        }
    }

    let pad = if compact { 3.0 } else { 4.0 };
    let avatar_cx = x + pad + avatar / 2.0;
    let avatar_cy = y + height / 2.0;
    draw_avatar(
        cr,
        avatars,
        participant,
        avatar_cx,
        avatar_cy,
        avatar,
        opacity,
        participant.speaking,
    );

    if compact {
        if muted || deafened {
            let badge_size = avatar * 0.5;
            let badge_cx = avatar_cx + avatar / 2.0 - badge_size * 0.12;
            let badge_cy = avatar_cy + avatar / 2.0 - badge_size * 0.12;
            cr.arc(badge_cx, badge_cy, badge_size / 2.0, 0.0, 2.0 * PI);
            cr.set_source_rgba(15.0 / 255.0, 16.0 / 255.0, 18.0 / 255.0, opacity);
            cr.fill_preserve();
            cr.set_source_rgba(42.0 / 255.0, 44.0 / 255.0, 49.0 / 255.0, opacity);
            cr.set_line_width(1.0);
            cr.stroke();
            draw_icon(
                cr,
                if deafened {
                    Icon::HeadphonesOff
                } else {
                    Icon::MicOff
                },
                badge_cx,
                badge_cy,
                badge_size * 0.55,
                (198.0 / 255.0, 200.0 / 255.0, 207.0 / 255.0, opacity),
            );
        }
        return;
    }

    let name_x = avatar_cx + avatar / 2.0 + 8.0;
    let right_padding = 10.0;
    let icon_size = font * 1.05;
    let mic_icon = muted;
    let deaf_icon = deafened;
    let bot_icon = participant.bot;
    let icon_count = usize::from(mic_icon) + usize::from(deaf_icon) + usize::from(bot_icon);
    let icons_width = if icon_count == 0 {
        0.0
    } else {
        icon_count as f64 * icon_size + (icon_count.saturating_sub(1)) as f64 * 2.0 + 8.0
    };
    let name_max = (x + width - right_padding - name_x - icons_width).max(10.0);

    if appearance.show_names {
        let name = truncate(
            &participant.display_name,
            appearance.max_name_length as usize,
        );
        let color = if muted {
            (179.0 / 255.0, 180.0 / 255.0, 189.0 / 255.0, opacity)
        } else {
            (233.0 / 255.0, 234.0 / 255.0, 238.0 / 255.0, opacity)
        };
        let text_width = draw_text(
            cr,
            &name,
            name_x,
            y + height / 2.0,
            font,
            color,
            name_max,
            None,
        );

        let mut icon_x = name_x + text_width.min(name_max) + 8.0 + icon_size / 2.0;
        let icon_cy = y + height / 2.0;
        if mic_icon {
            let color = if participant.muted {
                (242.0 / 255.0, 63.0 / 255.0, 67.0 / 255.0, opacity)
            } else {
                (183.0 / 255.0, 185.0 / 255.0, 194.0 / 255.0, opacity)
            };
            draw_icon(cr, Icon::MicOff, icon_x, icon_cy, icon_size, color);
            icon_x += icon_size + 2.0;
        }
        if deaf_icon {
            let color = if participant.deafened {
                (242.0 / 255.0, 63.0 / 255.0, 67.0 / 255.0, opacity)
            } else {
                (183.0 / 255.0, 185.0 / 255.0, 194.0 / 255.0, opacity)
            };
            draw_icon(cr, Icon::HeadphonesOff, icon_x, icon_cy, icon_size, color);
            icon_x += icon_size + 2.0;
        }
        if bot_icon {
            draw_icon(
                cr,
                Icon::Bot,
                icon_x,
                icon_cy,
                icon_size,
                (183.0 / 255.0, 185.0 / 255.0, 194.0 / 255.0, opacity),
            );
        }
    }

    let _ = scale;
}

fn draw_avatar(
    cr: &cairo::Context,
    avatars: &AvatarCache,
    participant: &Participant,
    cx: f64,
    cy: f64,
    size: f64,
    opacity: f64,
    speaking: bool,
) {
    let radius = size / 2.0;
    cr.save();
    cr.arc(cx, cy, radius, 0.0, 2.0 * PI);
    cr.clip();
    match avatars.images.get(&participant.avatar_url) {
        Some(pixbuf) if pixbuf.width() > 0 && pixbuf.height() > 0 => {
            let pixel_width = f64::from(pixbuf.width());
            let pixel_height = f64::from(pixbuf.height());
            let factor = (size / pixel_width).max(size / pixel_height);
            let draw_width = pixel_width * factor;
            let draw_height = pixel_height * factor;
            cr.save();
            cr.translate(cx - draw_width / 2.0, cy - draw_height / 2.0);
            cr.scale(factor, factor);
            use gdk::prelude::GdkContextExt;
            cr.set_source_pixbuf(pixbuf, 0.0, 0.0);
            cr.paint();
            cr.restore();
        }
        _ => {
            cr.set_source_rgba(43.0 / 255.0, 45.0 / 255.0, 49.0 / 255.0, opacity);
            cr.paint();
            let initial = participant
                .display_name
                .chars()
                .next()
                .unwrap_or('?')
                .to_uppercase()
                .to_string();
            let initial_width = measure_text(cr, &initial, size * 0.42);
            draw_text(
                cr,
                &initial,
                cx - initial_width / 2.0,
                cy - size * 0.02,
                size * 0.42,
                (0.86, 0.87, 0.89, opacity),
                size,
                None,
            );
        }
    }
    cr.restore();

    if speaking {
        cr.arc(cx, cy, radius + 1.5, 0.0, 2.0 * PI);
        cr.set_source_rgba(35.0 / 255.0, 165.0 / 255.0, 90.0 / 255.0, 1.0 * opacity);
        cr.set_line_width(2.0);
        cr.stroke();
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_empty_state(
    cr: &cairo::Context,
    view: &View,
    width: f64,
    top: f64,
    bottom: f64,
    radius: f64,
    font: f64,
    _scale: f64,
    opacity: f64,
    align_end: bool,
) {
    let height = font * 1.25 + 18.0;
    let y = if align_end {
        (bottom - height).max(top)
    } else {
        top
    };
    let x = 6.0;
    let w = width - 12.0;

    round_rect(cr, x + 0.5, y + 0.5, w - 1.0, height - 1.0, radius);
    cr.set_source_rgba(17.0 / 255.0, 18.0 / 255.0, 20.0 / 255.0, 0.72 * opacity);
    cr.fill_preserve();
    cr.set_dash(&[4.0, 3.0], 0.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.10 * opacity);
    cr.set_line_width(1.0);
    cr.stroke();
    cr.set_dash(&[], 0.0);

    draw_icon(
        cr,
        Icon::Mic,
        x + 18.0,
        y + height / 2.0,
        font * 1.1,
        (88.0 / 255.0, 101.0 / 255.0, 242.0 / 255.0, opacity),
    );
    let text = status_text(
        &view.status,
        view.overlay.channel.as_ref().map(|c| c.name.as_str()),
    );
    draw_text(
        cr,
        &text,
        x + 32.0,
        y + height / 2.0,
        font,
        (183.0 / 255.0, 185.0 / 255.0, 194.0 / 255.0, opacity),
        w - 40.0,
        None,
    );
}

fn status_text(status: &Status, channel: Option<&str>) -> String {
    match status {
        Status::Starting => "Starting…".to_string(),
        Status::WaitingForDiscord { .. } => "Waiting for Discord".to_string(),
        Status::Connecting => "Connecting to Discord".to_string(),
        Status::Authorizing => "Approve the prompt in Discord".to_string(),
        Status::Authenticating => "Connecting to Discord".to_string(),
        Status::Ready { .. } => match channel {
            Some(name) => format!("#{name} — nobody else is here yet"),
            None => "Not in a voice channel".to_string(),
        },
        Status::NeedsSetup { .. } => "Discord needs to be set up".to_string(),
        Status::Stopped => "Overlay stopped".to_string(),
    }
}

fn truncate(value: &str, max_length: usize) -> String {
    if value.chars().count() <= max_length {
        return value.to_string();
    }
    let keep = max_length.saturating_sub(1).max(1);
    let mut result: String = value.chars().take(keep).collect();
    result.push('…');
    result
}

fn draw_text(
    cr: &cairo::Context,
    text: &str,
    x: f64,
    center_y: f64,
    size: f64,
    color: (f64, f64, f64, f64),
    max_width: f64,
    _unused: Option<f64>,
) -> f64 {
    let layout = pangocairo::functions::create_layout(cr);
    let mut description = pango::FontDescription::new();
    description.set_family("Inter");
    description.set_absolute_size((size * f64::from(pango::SCALE)).max(1.0));
    layout.set_font_description(Some(&description));
    layout.set_ellipsize(pango::EllipsizeMode::End);
    layout.set_single_paragraph_mode(true);
    layout.set_width(((max_width.max(1.0)) * f64::from(pango::SCALE)) as i32);
    layout.set_text(text);
    let (width, height) = layout.pixel_size();
    cr.save();
    cr.move_to(x, center_y - f64::from(height) / 2.0);
    cr.set_source_rgba(color.0, color.1, color.2, color.3);
    pangocairo::functions::show_layout(cr, &layout);
    cr.restore();
    f64::from(width)
}

fn measure_text(cr: &cairo::Context, text: &str, size: f64) -> f64 {
    let layout = pangocairo::functions::create_layout(cr);
    let mut description = pango::FontDescription::new();
    description.set_family("Inter");
    description.set_absolute_size((size * f64::from(pango::SCALE)).max(1.0));
    layout.set_font_description(Some(&description));
    layout.set_text(text);
    f64::from(layout.pixel_size().0)
}

fn round_rect(cr: &cairo::Context, x: f64, y: f64, width: f64, height: f64, radius: f64) {
    let radius = radius.min(width / 2.0).min(height / 2.0).max(0.0);
    cr.new_sub_path();
    cr.arc(x + width - radius, y + radius, radius, -PI / 2.0, 0.0);
    cr.arc(
        x + width - radius,
        y + height - radius,
        radius,
        0.0,
        PI / 2.0,
    );
    cr.arc(x + radius, y + height - radius, radius, PI / 2.0, PI);
    cr.arc(x + radius, y + radius, radius, PI, 1.5 * PI);
    cr.close_path();
}

fn draw_icon(
    cr: &cairo::Context,
    icon: Icon,
    cx: f64,
    cy: f64,
    size: f64,
    color: (f64, f64, f64, f64),
) {
    let r = size / 2.0;
    cr.save();
    cr.set_source_rgba(color.0, color.1, color.2, color.3);
    cr.set_line_width((size * 0.09).max(1.2));
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);

    match icon {
        Icon::Mic => {
            let width = size * 0.28;
            let height = size * 0.44;
            round_rect(
                cr,
                cx - width / 2.0,
                cy - r * 0.95,
                width,
                height,
                width / 2.0,
            );
            cr.stroke();
            cr.arc(cx, cy - r * 0.05, size * 0.32, 0.0, PI);
            cr.stroke();
            cr.move_to(cx, cy + r * 0.27);
            cr.line_to(cx, cy + r * 0.55);
            cr.stroke();
        }
        Icon::MicOff => {
            draw_icon(cr, Icon::Mic, cx, cy, size, color);
            slash(cr, cx, cy, r * 0.8);
        }
        Icon::HeadphonesOff => {
            cr.arc(cx, cy + size * 0.02, size * 0.34, PI, 2.0 * PI);
            cr.stroke();
            cr.move_to(cx - size * 0.34, cy + size * 0.02);
            cr.line_to(cx - size * 0.34, cy + size * 0.2);
            cr.stroke();
            cr.move_to(cx + size * 0.34, cy + size * 0.02);
            cr.line_to(cx + size * 0.34, cy + size * 0.2);
            cr.stroke();
            slash(cr, cx, cy, r * 0.8);
        }
        Icon::Bot => {
            round_rect(
                cr,
                cx - size * 0.3,
                cy - size * 0.18,
                size * 0.6,
                size * 0.42,
                size * 0.08,
            );
            cr.stroke();
            cr.move_to(cx, cy - size * 0.18);
            cr.line_to(cx, cy - size * 0.36);
            cr.stroke();
            cr.arc(cx, cy - size * 0.4, size * 0.04, 0.0, 2.0 * PI);
            cr.fill();
        }
        Icon::Pointer | Icon::PointerOff => {
            cr.move_to(cx - size * 0.22, cy - size * 0.42);
            cr.line_to(cx + size * 0.24, cy + size * 0.06);
            cr.line_to(cx + size * 0.02, cy + size * 0.08);
            cr.line_to(cx + size * 0.14, cy + size * 0.38);
            cr.line_to(cx + size * 0.02, cy + size * 0.44);
            cr.line_to(cx - size * 0.1, cy + size * 0.14);
            cr.close_path();
            cr.fill_preserve();
            cr.stroke();
            if icon == Icon::PointerOff {
                slash(cr, cx, cy, r * 0.82);
            }
        }
        Icon::Lock | Icon::Unlock => {
            round_rect(
                cr,
                cx - size * 0.28,
                cy - size * 0.02,
                size * 0.56,
                size * 0.4,
                size * 0.08,
            );
            cr.stroke();
            let offset = if icon == Icon::Unlock {
                size * 0.12
            } else {
                0.0
            };
            cr.arc(cx + offset, cy - size * 0.02, size * 0.18, PI, 2.0 * PI);
            cr.stroke();
        }
        Icon::Refresh => {
            cr.arc(cx, cy, size * 0.3, -PI * 0.8, PI * 0.55);
            cr.stroke();
            let angle = PI * 0.55;
            let tip_x = cx + size * 0.3 * angle.cos();
            let tip_y = cy + size * 0.3 * angle.sin();
            cr.move_to(tip_x - size * 0.12, tip_y - size * 0.02);
            cr.line_to(tip_x, tip_y - size * 0.13);
            cr.line_to(tip_x + size * 0.08, tip_y + size * 0.02);
            cr.stroke();
        }
        Icon::Settings => {
            cr.arc(cx, cy, size * 0.16, 0.0, 2.0 * PI);
            cr.stroke();
            for index in 0..8 {
                let angle = f64::from(index) * PI / 4.0;
                cr.move_to(
                    cx + size * 0.24 * angle.cos(),
                    cy + size * 0.24 * angle.sin(),
                );
                cr.line_to(
                    cx + size * 0.38 * angle.cos(),
                    cy + size * 0.38 * angle.sin(),
                );
            }
            cr.stroke();
        }
        Icon::Grip => {
            for row in [-1.0, 1.0] {
                for column in [-1.0, 1.0] {
                    cr.arc(
                        cx + column * size * 0.14,
                        cy + row * size * 0.18,
                        size * 0.055,
                        0.0,
                        2.0 * PI,
                    );
                    cr.fill();
                }
            }
        }
    }
    cr.restore();
}

fn slash(cr: &cairo::Context, cx: f64, cy: f64, radius: f64) {
    cr.move_to(cx - radius, cy - radius);
    cr.line_to(cx + radius, cy + radius);
    cr.stroke();
}
