//! iPadOS: every file dialog is the Files app's document picker, presented over the window.
//!
//! - Open and Place pick files where they are (iCloud Drive, On My iPad, a file provider), not a
//!   copy: Save writes back to the same file. Access to each file picked is kept for the session.
//! - Save picks the folder to save in; the file keeps the name the app suggests.
//! - Folder pickers pick a folder.
//!
//! The app's own Documents folder is where the picker starts; with `UIFileSharingEnabled` and
//! `LSSupportsOpeningDocumentsInPlace` (Info.plist) it shows in Files as On My iPad › VectorCraft.
//!
//! UIKit's delegate protocol and the window's raw view have no safe API, hence the scoped
//! `unsafe_code` allowance.
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSArray, NSString, NSURL};
use objc2_ui_kit::{UIDocumentPickerDelegate, UIDocumentPickerViewController, UIView};
use objc2_uniform_type_identifiers::{UTType, UTTypeFolder, UTTypeItem};
use vectorcraft_ui_egui::FilePick;
use vectorcraft_ui_egui::picks::PickRequest;
use winit::raw_window_handle::{HasWindowHandle as _, RawWindowHandle};

use crate::Parent;

/// The app's Documents folder (shown in Files), made if missing.
pub fn documents_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("HOME")?).join("Documents");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

struct Ivars {
    answer: RefCell<Option<Sender<Vec<String>>>>,
    ui: egui::Context,
    /// Save: the file name, joined to the folder picked.
    save_as: Option<String>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "VectorCraftDocumentPickerDelegate"]
    #[ivars = Ivars]
    struct PickerDelegate;

    unsafe impl NSObjectProtocol for PickerDelegate {}

    unsafe impl UIDocumentPickerDelegate for PickerDelegate {
        #[unsafe(method(documentPicker:didPickDocumentsAtURLs:))]
        fn did_pick(&self, _picker: &UIDocumentPickerViewController, urls: &NSArray<NSURL>) {
            let paths = urls
                .iter()
                .filter_map(|url| {
                    // Kept for the session: the app reads the file, and writes it on Save, later.
                    let _ = unsafe { url.startAccessingSecurityScopedResource() };
                    Some(url.path()?.to_string())
                })
                .collect();
            self.answer(paths);
        }

        #[unsafe(method(documentPickerWasCancelled:))]
        fn cancelled(&self, _picker: &UIDocumentPickerViewController) {
            self.answer(Vec::new());
        }
    }
);

impl PickerDelegate {
    fn new(mtm: MainThreadMarker, ivars: Ivars) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }

    fn answer(&self, paths: Vec<String>) {
        let ivars = self.ivars();
        let paths = match &ivars.save_as {
            Some(name) => paths.into_iter().take(1).map(|folder| Path::new(&folder).join(name).to_string_lossy().into_owned()).collect(),
            None => paths,
        };
        if let Some(tx) = ivars.answer.borrow_mut().take() {
            // The app gone meanwhile has no use for the answer.
            let _ = tx.send(paths);
        }
        ivars.ui.request_repaint();
    }
}

thread_local! {
    /// The picker's delegate (the picker holds it weakly), until the next picker replaces it.
    static DELEGATE: RefCell<Option<Retained<PickerDelegate>>> = const { RefCell::new(None) };
}

/// The name a save dialog for `pick` suggests, with the extension of its default type.
fn save_name(pick: &FilePick) -> String {
    let ext = pick.filters.first().and_then(|(_, exts)| exts.first()).copied();
    let name = if pick.name.is_empty() { "Untitled" } else { pick.name.as_str() };
    match ext {
        Some(ext) if Path::new(name).extension().is_none() => format!("{name}.{ext}"),
        _ => name.to_string(),
    }
}

/// Show the document picker for `request` over `parent`'s window, answering on the receiver.
pub fn start_pick(request: PickRequest, parent: &Parent, ui: &egui::Context) -> Option<Receiver<Vec<String>>> {
    let Some(mtm) = MainThreadMarker::new() else {
        log::error!("no document picker for {request:?}: not on the main thread");
        return None;
    };
    let window = parent.as_ref()?;
    let RawWindowHandle::UiKit(handle) = window.window_handle().ok()?.as_raw() else {
        return None;
    };
    // SAFETY: winit's UIKit handle is the window's UIView, alive as long as the window is.
    let view: &UIView = unsafe { handle.ui_view.cast::<UIView>().as_ref() };
    let mut presenter = view.window()?.rootViewController()?;
    while let Some(next) = presenter.presentedViewController() {
        presenter = next;
    }

    let (folder_types, multiple, save_as, start) = match &request {
        PickRequest::Open(pick) => (false, false, None, pick.folder.clone()),
        PickRequest::OpenMany => (false, true, None, None),
        PickRequest::Save(pick) => (true, false, Some(save_name(pick)), pick.folder.clone()),
        PickRequest::Folder => (true, false, None, None),
    };
    let start = start.map(PathBuf::from).filter(|p| p.is_dir()).or_else(documents_dir);
    let kind: &UTType = unsafe { if folder_types { UTTypeFolder } else { UTTypeItem } };
    let types = NSArray::from_slice(&[kind]);
    let picker = UIDocumentPickerViewController::initForOpeningContentTypes_asCopy(UIDocumentPickerViewController::alloc(mtm), &types, false);
    picker.setAllowsMultipleSelection(multiple);
    picker.setShouldShowFileExtensions(true);
    if let Some(start) = start {
        picker.setDirectoryURL(Some(&NSURL::fileURLWithPath(&NSString::from_str(&start.to_string_lossy()))));
    }

    let (tx, rx) = channel();
    let delegate = PickerDelegate::new(mtm, Ivars { answer: RefCell::new(Some(tx)), ui: ui.clone(), save_as });
    picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    DELEGATE.with(|d| *d.borrow_mut() = Some(delegate));
    presenter.presentViewController_animated_completion(&picker, true, None);
    Some(rx)
}

/// The memory and CPU VectorCraft uses, and the battery level, for the header readout. Cheap
/// (two system calls and two UIDevice properties); the CPU share is measured since the previous
/// call. The battery needs the main thread (unknown off it).
pub fn resources() -> vectorcraft_ui_egui::ResourceReading {
    use std::sync::Mutex;
    use std::time::Instant;
    static LAST: Mutex<Option<(Instant, f64)>> = Mutex::new(None);

    let mut out = vectorcraft_ui_egui::ResourceReading::default();
    // The footprint is what Xcode's memory gauge and iPadOS's memory limit use.
    let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    if unsafe { libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V2, (&raw mut info).cast()) } == 0 {
        out.memory_bytes = info.ri_phys_footprint;
    }
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut usage) } == 0 {
        let secs = |t: libc::timeval| t.tv_sec as f64 + f64::from(t.tv_usec) / 1e6;
        let cpu = secs(usage.ru_utime) + secs(usage.ru_stime);
        let now = Instant::now();
        let mut last = LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((then, before)) = *last {
            let wall = now.duration_since(then).as_secs_f64();
            let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get) as f64;
            if wall > 0.0 {
                out.cpu_percent = ((cpu - before) / wall / cores * 100.0).clamp(0.0, 100.0) as f32;
            }
        }
        *last = Some((now, cpu));
    }
    if let Some(mtm) = MainThreadMarker::new() {
        let device = objc2_ui_kit::UIDevice::currentDevice(mtm);
        if !device.isBatteryMonitoringEnabled() {
            device.setBatteryMonitoringEnabled(true);
        }
        let level = device.batteryLevel();
        if level >= 0.0 {
            out.battery_percent = Some(level * 100.0);
        }
        let state = device.batteryState();
        out.charging = state == objc2_ui_kit::UIDeviceBatteryState::Charging || state == objc2_ui_kit::UIDeviceBatteryState::Full;
    }
    out
}
