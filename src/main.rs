// Core GTK4/Libadwaita application for analyzing DR values in text files
use libadwaita::prelude::*;
use anyhow::Result;
use gtk4::{
    gio, glib,
    glib::clone,
    PolicyType, ScrolledWindow,
};
use once_cell::sync::Lazy;
use rayon::prelude::*;
use regex::Regex;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    process::Command,
};
use walkdir::WalkDir;
use gtk4::gdk::{Key, ModifierType};
use gtk4::glib::Propagation;
use async_channel::bounded;

// Regex pattern matches both English and Russian DR value formats
// Format: "Official DR value: DR12" or "Реальные значения DR: DR12"
static DR_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"Official DR value:\s*DR(\d+|ERR)|Реальные значения DR:\s*DR(\d+|ERR)").unwrap()
});

// Color mapping for DR values visualization:
// DR0-7: Red - Critical to severe issues
// DR8: #ff4800 - Dark orange
// DR9: #ff9100 - Orange
// DR10: #ffd900 - Yellow
// DR11: #d9ff00 - Lime
// DR12: #90ff00 - Light green
// DR13: #48ff00 - Green
// DR14: #0f0 - Bright green
const DR_COLORS: [(u8, u8, u8); 15] = [
    (255, 0, 0),     // DR0-7 - Red
    (255, 0, 0),     (255, 0, 0),     (255, 0, 0),     (255, 0, 0),     (255, 0, 0),     (255, 0, 0),     (255, 0, 0),
    (255, 72, 0),    // DR8 - #ff4800
    (255, 145, 0),   // DR9 - #ff9100
    (255, 217, 0),   // DR10 - #ffd900
    (217, 255, 0),   // DR11 - #d9ff00
    (144, 255, 0),   // DR12 - #90ff00
    (72, 255, 0),    // DR13 - #48ff00
    (0, 255, 0),     // DR14 - #0f0
];

#[derive(Debug, Clone)]
struct DRResult {
    filename: String,
    path: PathBuf,
    dr_value: Option<u8>,  // None represents ERR or unscanned state
    scanned: bool,         // Distinguishes between ERR (scanned) and PENDING (unscanned)
}

struct AppState {
    results: Vec<DRResult>,
    delete_files: bool,  // Whether to delete files from system when removing from list
    delete_folders: bool,  // Whether to delete parent folders when deleting files
}

// Opens file in system's default application, showing error dialog on failure
fn try_open_file(window: &libadwaita::ApplicationWindow, path: &Path) {
    if !path.exists() {
        show_error_dialog(window, &format!("File not found: {}", path.display()));
        return;
    }

    // SAFETY: xdg-open is a standard command that's safe to execute
    if let Err(err) = Command::new("xdg-open").arg(path).spawn() {
        show_error_dialog(window, &format!("Failed to open file: {}", err));
    }
}

fn show_error_dialog(window: &libadwaita::ApplicationWindow, message: &str) {
    let dialog = gtk4::MessageDialog::new(
        Some(window),
        gtk4::DialogFlags::MODAL,
        gtk4::MessageType::Error,
        gtk4::ButtonsType::Ok,
        message
    );
    // Auto-close dialog on response to prevent memory leaks
    dialog.connect_response(|dialog, _| dialog.close());
    dialog.show();
}

// Creates a three-column view for displaying file analysis results:
// 1. Filename (300px)
// 2. Full path (700px)
// 3. DR Value with color indicator (flexible width)
fn create_column_view() -> (gtk4::ColumnView, gio::ListStore, gtk4::MultiSelection) {
    let list_store = gio::ListStore::new::<gtk4::StringObject>();
    let selection_model = gtk4::MultiSelection::new(Some(list_store.clone()));
    let column_view = gtk4::ColumnView::new(Some(selection_model.clone()));
    
    // Enable rubber band selection and row separators for better UX
    column_view.set_show_row_separators(true);
    column_view.set_enable_rubberband(true);
    column_view.set_hexpand(true);
    column_view.set_vexpand(true);
    column_view.set_valign(gtk4::Align::Fill);

    add_column(&column_view, "File Name", 300, |text| text.split('\t').next().unwrap_or(""));
    add_column(&column_view, "Path", 700, |text| text.split('\t').nth(1).unwrap_or(""));
    add_dr_column(&column_view);

    (column_view, list_store, selection_model)
}

// Adds a text column with custom text extraction logic
fn add_column(column_view: &gtk4::ColumnView, title: &str, width: i32, text_extractor: impl Fn(&str) -> &str + 'static) {
    let factory = gtk4::SignalListItemFactory::new();
    
    // Setup handler creates label with consistent styling
    factory.connect_setup(move |_, list_item| {
        let label = gtk4::Label::new(None);
        // Left-align text and add margins for better readability
        label.set_xalign(0.0);
        label.set_margin_start(5);
        label.set_margin_end(5);
        // Truncate long text with ellipsis
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        label.set_width_chars(30);
        list_item.set_child(Some(&label));
    });

    // Bind handler updates label text using the provided extractor
    factory.connect_bind(move |_, list_item| {
        let string_object = list_item.item().and_downcast::<gtk4::StringObject>().unwrap();
        let label = list_item.child().and_downcast::<gtk4::Label>().unwrap();
        let text = string_object.string();
        label.set_text(text_extractor(&text));
    });

    let column = gtk4::ColumnViewColumn::new(Some(title), Some(factory));
    column.set_resizable(true);
    column.set_expand(false);
    column.set_fixed_width(width);
    column_view.append_column(&column);
}

// Adds a special column for DR values with color indicators
fn add_dr_column(column_view: &gtk4::ColumnView) {
    let factory = gtk4::SignalListItemFactory::new();
    
    // Setup handler creates a horizontal box with color indicator and label
    factory.connect_setup(move |_, list_item| {
        let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 5);
        let color_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        let label = gtk4::Label::new(None);
        
        // Color indicator box with fixed size
        color_box.set_size_request(16, 16);
        color_box.add_css_class("color-box");
        
        hbox.append(&color_box);
        hbox.append(&label);
        hbox.set_spacing(5);
        list_item.set_child(Some(&hbox));
    });

    // Bind handler updates DR value and color indicator
    factory.connect_bind(move |_, list_item| {
        let string_object = list_item.item().and_downcast::<gtk4::StringObject>().unwrap();
        let hbox = list_item.child().and_downcast::<gtk4::Box>().unwrap();
        let color_box = hbox.first_child().and_downcast::<gtk4::Box>().unwrap();
        let label = hbox.last_child().and_downcast::<gtk4::Label>().unwrap();

        let text = string_object.string();
        let dr_text = text.split('\t').nth(2).unwrap_or("PENDING");
        label.set_text(dr_text);
        
        // Map DR values to colors: PENDING=gray, ERR=dark gray, numeric values use DR_COLORS
        let (r, g, b) = match dr_text {
            "PENDING" => (180, 180, 180),
            "ERR" => (128, 128, 128),
            _ => dr_text.parse::<u8>()
                .map(|dr| if dr < DR_COLORS.len() as u8 { DR_COLORS[dr as usize] } else { (128, 128, 128) })
                .unwrap_or((128, 128, 128))
        };
        
        // Apply color to indicator box using CSS
        let css = format!("box.color-box {{ background-color: rgb({}, {}, {}); }}", r, g, b);
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_data(&css);
        color_box.style_context().add_provider(&css_provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
    });

    let column = gtk4::ColumnViewColumn::new(Some("DR Value"), Some(factory));
    column.set_resizable(true);
    column.set_expand(true);
    column_view.append_column(&column);
}

// Shows the settings dialog
fn show_settings_dialog(window: &libadwaita::ApplicationWindow, app_state: &Arc<Mutex<AppState>>) {
    let dialog = gtk4::Dialog::new();
    dialog.set_title(Some("Settings"));
    dialog.set_transient_for(Some(window));
    dialog.set_modal(true);
    dialog.set_default_size(400, 300);

    let content_area = dialog.content_area();
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    // Add margins for better spacing in Libadwaita
    vbox.set_margin_top(10);
    vbox.set_margin_bottom(10);
    vbox.set_margin_start(10);
    vbox.set_margin_end(10);

    // Add delete files setting
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    let label = gtk4::Label::new(Some("Delete files from system when removing from list"));
    label.set_hexpand(true);
    label.set_xalign(0.0);

    let switch = gtk4::Switch::new();
    let folder_switch = gtk4::Switch::new();  // Create folder switch early
    
    // Initialize switch states
    if let Ok(state) = app_state.lock() {
        switch.set_active(state.delete_files);
        folder_switch.set_active(false);  // Always start off
        folder_switch.set_sensitive(state.delete_files);
    }

    hbox.append(&label);
    hbox.append(&switch);
    vbox.append(&hbox);

    // Add delete folders setting
    let folder_hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    let folder_label = gtk4::Label::new(Some("Also delete parent folders (DANGEROUS)"));
    folder_label.set_hexpand(true);
    folder_label.set_xalign(0.0);
    folder_label.add_css_class("dim-label");

    folder_hbox.append(&folder_label);
    folder_hbox.append(&folder_switch);
    vbox.append(&folder_hbox);

    // Handle main switch state changes
    switch.connect_state_set(clone!(@strong app_state, @strong folder_switch, @strong folder_label => move |_, active| {
        if let Ok(mut state) = app_state.lock() {
            // Update main switch state first
            state.delete_files = active;
            
            // If turning off main switch, ensure folder switch is off
            if !active {
                // Update the state first
                state.delete_folders = false;
                
                // Then update UI after releasing the lock
                glib::idle_add_local_once(clone!(@strong folder_switch, @strong folder_label => move || {
                    folder_switch.set_active(false);
                    folder_switch.set_sensitive(false);
                    folder_label.add_css_class("dim-label");
                }));
            } else {
                // If turning on main switch, just enable folder switch
                glib::idle_add_local_once(clone!(@strong folder_switch, @strong folder_label => move || {
                    folder_switch.set_sensitive(true);
                    folder_label.remove_css_class("dim-label");
                }));
            }
        }
        Propagation::Proceed
    }));

    // Handle folder switch state changes
    folder_switch.connect_state_set(clone!(@strong app_state => move |_, active| {
        if let Ok(mut state) = app_state.lock() {
            if state.delete_files {
                state.delete_folders = active;
                Propagation::Proceed
            } else {
                // If main switch is off, prevent any state change
                Propagation::Stop
            }
        } else {
            Propagation::Stop
        }
    }));

    content_area.append(&vbox);
    dialog.show();
}

fn build_ui(app: &libadwaita::Application) {
    let window = libadwaita::ApplicationWindow::new(app);
    window.set_title(Some("DR Analyzer"));
    window.set_default_size(1200, 600);
    window.set_resizable(true);
    window.set_size_request(1000, -1);

    // Create header bar with action buttons
    let header_bar = libadwaita::HeaderBar::new();
    let open_button = gtk4::Button::with_label("Select Directory");
    let scan_button = gtk4::Button::with_label("Scan Files");
    let clear_button = gtk4::Button::with_label("Clear List");
    
    // Add settings button with gear icon
    let settings_button = gtk4::Button::from_icon_name("system-run-symbolic");
    settings_button.set_tooltip_text(Some("Settings"));
    
    // Buttons start disabled until directory is selected
    scan_button.set_sensitive(false);
    clear_button.set_sensitive(false);
    scan_button.add_css_class("suggested-action");
    
    header_bar.pack_start(&open_button);
    header_bar.pack_end(&settings_button);
    header_bar.pack_end(&clear_button);
    header_bar.pack_end(&scan_button);

    // Main vertical layout
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    vbox.append(&header_bar);

    // Progress bar (hidden by default)
    let progress_bar = gtk4::ProgressBar::new();
    progress_bar.set_visible(false);
    vbox.append(&progress_bar);

    // Scrollable view for results
    let scrolled = ScrolledWindow::new();
    scrolled.set_hexpand(true);
    scrolled.set_vexpand(true);
    scrolled.set_policy(PolicyType::Never, PolicyType::Automatic);
    scrolled.set_propagate_natural_height(true);
    scrolled.set_valign(gtk4::Align::Fill);

    let (column_view, list_store, selection_model) = create_column_view();
    let viewport = gtk4::Viewport::new(None::<&gtk4::Adjustment>, None::<&gtk4::Adjustment>);
    viewport.set_hexpand(true);
    viewport.set_vexpand(true);
    viewport.set_valign(gtk4::Align::Fill);
    viewport.set_child(Some(&column_view));
    scrolled.set_child(Some(&viewport));
    vbox.append(&scrolled);
    window.set_content(Some(&vbox));

    // Shared state for managing results and selected directory
    let app_state = Arc::new(Mutex::new(AppState { 
        results: Vec::new(),
        delete_files: false,  // Default to not deleting files
        delete_folders: false,  // Default to not deleting folders
    }));
    let selected_path = Arc::new(Mutex::new(None::<PathBuf>));

    // Set up event handlers
    setup_keyboard_controls(&window, &selection_model, &list_store, &app_state);
    setup_mouse_controls(&column_view, &window, &selection_model);
    setup_button_actions(&window, &open_button, &scan_button, &clear_button, &selected_path, 
                        &list_store, &app_state, &progress_bar);

    // Connect settings button
    settings_button.connect_clicked(clone!(@weak window, @strong app_state => move |_| {
        show_settings_dialog(&window, &app_state);
    }));

    window.present();
}

// Sets up keyboard shortcuts:
// - Ctrl+A: Select all items
// - Delete: Remove selected items
// - Enter: Open selected files
fn setup_keyboard_controls(window: &libadwaita::ApplicationWindow, selection_model: &gtk4::MultiSelection, 
                         list_store: &gio::ListStore, app_state: &Arc<Mutex<AppState>>) {
    let key_controller = gtk4::EventControllerKey::new();
    // Capture phase ensures we handle events before other widgets
    key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
    window.add_controller(key_controller.clone());
    
    key_controller.connect_key_pressed(clone!(@weak window, @weak selection_model, @weak list_store, @weak app_state => 
        @default-return Propagation::Proceed, move |_controller, key, _keycode, modifier_state| {
            match key {
                // Ctrl+A: Select all items in the list
                Key::a | Key::A if modifier_state.bits() & ModifierType::CONTROL_MASK.bits() != 0 => {
                    selection_model.unselect_all();
                    for i in 0..selection_model.n_items() {
                        selection_model.select_item(i, false);
                    }
                    Propagation::Stop
                }
                // Delete: Remove selected items from list and optionally from filesystem
                Key::Delete => {
                    delete_selected_files(&window, &selection_model, &list_store, &app_state);
                    Propagation::Stop
                }
                // Enter: Open selected files in default application
                Key::Return | Key::KP_Enter | Key::ISO_Enter => {
                    let selected_indices: Vec<u32> = (0..selection_model.n_items())
                        .filter(|&i| selection_model.is_selected(i))
                        .collect();

                    if !selected_indices.is_empty() {
                        for index in selected_indices {
                            if let Some(item) = selection_model.item(index) {
                                if let Some(string_obj) = item.downcast_ref::<gtk4::StringObject>() {
                                    let path = std::path::PathBuf::from(string_obj.string().split('\t').nth(1).unwrap_or(""));
                                    try_open_file(&window, &path);
                                }
                            }
                        }
                        Propagation::Stop
                    } else {
                        Propagation::Proceed
                    }
                }
                _ => Propagation::Proceed
            }
    }));
}

// Sets up double-click handler to open files
fn setup_mouse_controls(column_view: &gtk4::ColumnView, window: &libadwaita::ApplicationWindow, 
                       selection_model: &gtk4::MultiSelection) {
    let gesture_click = gtk4::GestureClick::new();
    gesture_click.set_button(1);  // Left mouse button
    column_view.add_controller(gesture_click.clone());
    
    // Capture phase ensures we handle events before other widgets
    gesture_click.set_propagation_phase(gtk4::PropagationPhase::Capture);
    gesture_click.connect_released(clone!(@weak window, @weak selection_model => move |gesture, n_press, _x, _y| {
        // Double-click (n_press == 2) on left button (button 1)
        if gesture.current_button() == 1 && n_press == 2 {
            if let Some(index) = (0..selection_model.n_items()).find(|&i| selection_model.is_selected(i)) {
                if let Some(item) = selection_model.item(index) {
                    if let Some(string_obj) = item.downcast_ref::<gtk4::StringObject>() {
                        let path = std::path::PathBuf::from(string_obj.string().split('\t').nth(1).unwrap_or(""));
                        try_open_file(&window, &path);
                    }
                }
            }
        }
    }));
}

// Sets up main button actions and their interdependencies:
// - Open button: Select directory and populate initial file list
// - Scan button: Analyze DR values in selected files
// - Clear button: Reset all results
fn setup_button_actions(window: &libadwaita::ApplicationWindow, open_button: &gtk4::Button, 
                       scan_button: &gtk4::Button, clear_button: &gtk4::Button, 
                       selected_path: &Arc<Mutex<Option<PathBuf>>>, list_store: &gio::ListStore, 
                       app_state: &Arc<Mutex<AppState>>, progress_bar: &gtk4::ProgressBar) {
    // Update button states based on list store contents
    list_store.connect_items_changed(clone!(@weak scan_button, @weak clear_button => move |list_store, _, _, _| {
        let has_items = list_store.n_items() > 0;
        clear_button.set_sensitive(has_items);
        scan_button.set_sensitive(has_items);
    }));

    // Clear button resets all state
    clear_button.connect_clicked(clone!(@strong list_store, @strong app_state, @strong scan_button, @strong clear_button => move |_| {
        list_store.remove_all();
        if let Ok(mut state) = app_state.lock() {
            state.results.clear();
        }
        scan_button.set_sensitive(false);
        clear_button.set_sensitive(false);
    }));

    // Open button shows directory selection dialog
    open_button.connect_clicked(clone!(@strong window, @strong scan_button, @strong clear_button, @strong selected_path, @strong list_store, @strong app_state => move |_| {
        let dialog = gtk4::FileChooserDialog::new(
            Some("Select Directory"),
            Some(&window),
            gtk4::FileChooserAction::SelectFolder,
            &[("Cancel", gtk4::ResponseType::Cancel), ("Open", gtk4::ResponseType::Accept)]
        );

        dialog.connect_response(clone!(@strong scan_button, @strong clear_button, @strong selected_path, @strong list_store, @strong app_state => move |dialog, response| {
            if response == gtk4::ResponseType::Accept {
                if let Some(path) = dialog.file().and_then(|f| f.path()) {
                    *selected_path.lock().unwrap() = Some(path.clone());
                    
                    // Find all .txt and .log files in selected directory
                    let files: Vec<_> = WalkDir::new(&path)
                        .into_iter()
                        .filter_map(Result::ok)
                        .filter(|entry| {
                            entry.file_type().is_file() && entry
                                .path()
                                .extension()
                                .map(|ext| ext == "txt" || ext == "log")
                                .unwrap_or(false)
                        })
                        .collect();

                    let initial_results: Vec<DRResult> = files.iter().map(|entry| DRResult {
                        filename: entry.file_name().to_string_lossy().into_owned(),
                        path: entry.path().to_path_buf(),
                        dr_value: None,
                        scanned: false,
                    }).collect();

                    if let Ok(mut state) = app_state.lock() {
                        state.results = initial_results;
                        update_ui(&list_store, &state.results);
                        
                        let has_items = !state.results.is_empty();
                        scan_button.set_sensitive(has_items);
                        clear_button.set_sensitive(has_items);
                    }
                }
            }
            dialog.close();
        }));

        dialog.show();
    }));

    // Scan button initiates DR value analysis
    scan_button.connect_clicked(clone!(@strong app_state, @strong progress_bar, @strong list_store, @strong selected_path, @strong clear_button => move |button| {
        if let Some(path) = selected_path.lock().unwrap().clone() {
            button.set_sensitive(false);
            clear_button.set_sensitive(false);
            progress_bar.set_visible(true);
            progress_bar.set_fraction(0.0);
            
            scan_directory(path, app_state.clone(), progress_bar.clone(), list_store.clone(), button.clone(), clear_button.clone());
        }
    }));
}

// Performs asynchronous directory scanning with progress updates
fn scan_directory(path: PathBuf, app_state: Arc<Mutex<AppState>>, progress_bar: gtk4::ProgressBar, 
                 list_store: gio::ListStore, scan_button: gtk4::Button, clear_button: gtk4::Button) {
    // Create bounded channels for progress updates and results
    let (progress_tx, progress_rx) = bounded::<(usize, usize)>(100);
    let (results_tx, results_rx) = bounded::<Vec<DRResult>>(1);
    
    // Spawn worker thread for file analysis
    std::thread::spawn(move || {
        // Find all .txt and .log files in selected directory
        let files: Vec<_> = WalkDir::new(path.clone())
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.file_type().is_file() && entry
                    .path()
                    .extension()
                    .map(|ext| ext == "txt" || ext == "log")
                    .unwrap_or(false)
            })
            .collect();

        let total_files = files.len();
        if total_files == 0 {
            progress_tx.send_blocking((0, 0)).expect("Channel send failed");
            results_tx.send_blocking(Vec::new()).expect("Failed to send empty results");
            return;
        }

        // Process files in parallel using rayon
        let results: Vec<DRResult> = files
            .par_iter()
            .enumerate()
            .map(|(i, entry)| {
                let result = analyze_file(entry.path());
                // Send progress update after each file
                progress_tx.send_blocking((i + 1, total_files)).expect("Channel send failed");
                result
            })
            .collect();

        results_tx.send_blocking(results).expect("Failed to send results");
    });

    // Handle progress updates in UI
    glib::MainContext::default().spawn_local(clone!(@strong progress_bar => async move {
        while let Ok((current, total)) = progress_rx.recv().await {
            if total > 0 {
                progress_bar.set_fraction(current as f64 / total as f64);
            }
        }
    }));

    // Handle final results
    glib::MainContext::default().spawn_local(clone!(@strong list_store, @strong app_state, @strong progress_bar, @strong scan_button, @strong clear_button => async move {
        if let Ok(results) = results_rx.recv().await {
            if let Ok(mut state) = app_state.lock() {
                state.results = results;
                update_ui(&list_store, &state.results);
            }
            
            progress_bar.set_visible(false);
            scan_button.set_sensitive(true);
            clear_button.set_sensitive(true);
        }
    }));
}

// Analyzes a single file for DR value
fn analyze_file(path: &Path) -> DRResult {
    // Read file content with UTF-8 fallback
    let content = match std::fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => return create_error_result(path),
    };

    // Extract DR value using regex pattern
    let dr_value = DR_REGEX
        .captures(&content)
        .and_then(|caps| {
            caps.get(1)
                .or_else(|| caps.get(2))
                .map(|m| m.as_str())
                .and_then(|val| {
                    if val == "ERR" {
                        None
                    } else {
                        val.parse::<u8>().ok()
                    }
                })
        });

    DRResult {
        filename: path.file_name().unwrap().to_string_lossy().into_owned(),
        path: path.to_path_buf(),
        dr_value,
        scanned: true,
    }
}

fn create_error_result(path: &Path) -> DRResult {
    DRResult {
        filename: path.file_name().unwrap().to_string_lossy().into_owned(),
        path: path.to_path_buf(),
        dr_value: None,
        scanned: true,
    }
}

// Updates UI with sorted results:
// - DR values are sorted in descending order (highest first)
// - Files with same DR value are sorted by path alphabetically
// - Files with errors are grouped together
// - Unscanned files are shown last
fn update_ui(list_store: &gio::ListStore, results: &[DRResult]) {
    let results = results.to_vec();
    let list_store = list_store.clone();
    
    // Use GLib's main context to update UI from background thread
    glib::MainContext::default().invoke_local(move || {
        list_store.remove_all();
        
        let mut sorted_results = results;
        sorted_results.sort_by(|a, b| {
            match (a.dr_value, b.dr_value) {
                (Some(a_val), Some(b_val)) => b_val.cmp(&a_val)
                    .then_with(|| a.path.cmp(&b.path)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => match (a.scanned, b.scanned) {
                    (true, true) | (false, false) => a.path.cmp(&b.path),
                    (true, false) => std::cmp::Ordering::Greater,
                    (false, true) => std::cmp::Ordering::Less,
                }
            }
        });

        for result in sorted_results {
            let dr_text = match (result.dr_value, result.scanned) {
                (Some(dr), _) => dr.to_string(),
                (None, true) => "ERR".to_string(),
                (None, false) => "PENDING".to_string(),
            };

            let text = format!(
                "{}\t{}\t{}",
                result.filename,
                result.path.to_string_lossy(),
                dr_text
            );
            list_store.append(&gtk4::StringObject::new(&text));
        }
    });
}

// Removes selected files from the list and optionally from the filesystem
fn delete_selected_files(window: &libadwaita::ApplicationWindow, selection_model: &gtk4::MultiSelection, 
                        list_store: &gio::ListStore, app_state: &Arc<Mutex<AppState>>) {
    let selected_items: Vec<_> = (0..selection_model.n_items())
        .filter(|&i| selection_model.is_selected(i))
        .collect();

    if selected_items.is_empty() {
        return;
    }

    let mut paths_to_remove = Vec::new();

    if let Ok(_) = app_state.lock() {
        for &index in &selected_items {
            if let Some(item) = selection_model.item(index) {
                if let Some(string_obj) = item.downcast_ref::<gtk4::StringObject>() {
                    let path = std::path::PathBuf::from(string_obj.string().split('\t').nth(1).unwrap_or(""));
                    paths_to_remove.push(path);
                }
            }
        }
    }

    // Check if we need to show confirmation dialog
    let should_confirm = if let Ok(state) = app_state.lock() {
        state.delete_files
    } else {
        false
    };

    if should_confirm {
        let dialog = gtk4::MessageDialog::new(
            Some(window),
            gtk4::DialogFlags::MODAL,
            gtk4::MessageType::Warning,
            gtk4::ButtonsType::YesNo,
            &format!("This will permanently delete {} file(s) from your system{} Continue?", 
                paths_to_remove.len(),
                if let Ok(state) = app_state.lock() {
                    if state.delete_folders { " and their parent folders" } else { "" }
                } else { "" }
            )
        );

        // Style the Yes button with a red tone
        if let Some(button) = dialog.widget_for_response(gtk4::ResponseType::Yes) {
            button.add_css_class("destructive-action");
        }

        let paths_to_remove_clone = paths_to_remove.clone();
        dialog.connect_response(clone!(@strong app_state, @strong list_store => move |dialog, response| {
            if response == gtk4::ResponseType::Yes {
                if let Ok(mut state) = app_state.lock() {
                    // Delete files from system
                    for path in &paths_to_remove_clone {
                        if let Err(err) = std::fs::remove_file(path) {
                            eprintln!("Failed to delete file {}: {}", path.display(), err);
                        } else if state.delete_folders {
                            // Try to remove parent folder if it's empty
                            if let Some(parent) = path.parent() {
                                if let Ok(entries) = std::fs::read_dir(parent) {
                                    if entries.count() == 0 {
                                        if let Err(err) = std::fs::remove_dir(parent) {
                                            eprintln!("Failed to delete empty folder {}: {}", parent.display(), err);
                                        }
                                    }
                                }
                            }
                        }
                    }

                    state.results.retain(|result| !paths_to_remove_clone.contains(&result.path));
                    update_ui(&list_store, &state.results);
                }
            }
            dialog.close();
        }));

        dialog.show();
    } else {
        if let Ok(mut state) = app_state.lock() {
            state.results.retain(|result| !paths_to_remove.contains(&result.path));
            update_ui(&list_store, &state.results);
        }
    }
}

fn main() -> Result<()> {
    // Initialize Libadwaita for modern GNOME look and feel
    libadwaita::init().expect("Failed to initialize libadwaita");

    let application = libadwaita::Application::new(
        Some("com.example.dr_analyzer"),
        gio::ApplicationFlags::FLAGS_NONE,
    );

    application.connect_activate(build_ui);
    application.run();

    Ok(())
}
