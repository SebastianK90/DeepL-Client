use super::language_popup::NativeChoice;
use super::{AppState, language_popup, lock};
use crate::core::{config::WorkMode, popup::Anchor};
#[cfg(target_os = "linux")]
use gtk::prelude::*;
use tauri::{Emitter, Manager};

thread_local! { static POPOVER: std::cell::RefCell<Option<gtk::Popover>> = const { std::cell::RefCell::new(None) }; }
pub(super) fn is_open() -> bool {
    POPOVER.with(|slot| slot.borrow().as_ref().is_some_and(|p| p.is_mapped()))
}
pub(super) fn close() {
    let old = POPOVER.with(|slot| slot.borrow_mut().take());
    if let Some(popover) = old {
        popover.popdown();
        unsafe {
            popover.destroy();
        }
    }
}
pub(super) async fn open(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    mode: WorkMode,
    anchor: Anchor,
    choices: Vec<NativeChoice>,
    placeholder: String,
) -> Result<u64, String> {
    if window.label() != "quick"
        || choices.len() > crate::core::LANGUAGES.len()
        || placeholder.len() > 300
    {
        return Err("Ungültiges Fenster.".into());
    }
    for item in &choices {
        if item.label.chars().count() > 120
            || item.code == "AUTO"
            || !crate::core::LANGUAGES.contains(&item.code.as_str())
            || (mode == WorkMode::Write
                && !crate::core::WRITE_LANGUAGES.contains(&item.code.as_str()))
        {
            return Err("Ungültige Zielsprache.".into());
        }
    }
    if [anchor.x, anchor.y, anchor.width, anchor.height]
        .iter()
        .any(|v| !v.is_finite())
        || anchor.x < 0.
        || anchor.y < 0.
        || anchor.width <= 0.
        || anchor.height <= 0.
    {
        return Err("Ungültiger Ziehbereich.".into());
    }
    let origin = tauri::async_runtime::spawn_blocking(super::kwin_geometry::quick_origin)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("KDE-Fensterposition nicht verfügbar.")?;
    let config = lock(&app.state::<AppState>().config)?.clone();
    let revision = language_popup::open_native(&app, mode)?;
    let selected = if mode == WorkMode::Write {
        config.write_language
    } else {
        config.target
    };
    let dark = config.theme == "dark";
    let system_theme = config.theme == "system";
    let owner = app.clone();
    let opened = window.with_webview(move |platform| {
        close();
        let dark = dark || (system_theme && gtk::Settings::default().is_some_and(|s| s.is_gtk_application_prefer_dark_theme()));
        let view = platform.inner();
        let popover = gtk::Popover::new(Some(&view));
        #[cfg(debug_assertions)]
        {
            eprintln!("Picker {revision}: owner mapped={}, size={}x{}",view.is_mapped(),view.allocated_width(),view.allocated_height());
            popover.connect_map(move |_| eprintln!("Picker {revision}: mapped"));
            popover.connect_unmap(move |_| eprintln!("Picker {revision}: unmapped"));
            popover.connect_closed(move |_| eprintln!("Picker {revision}: closed"));
        }
        popover.set_widget_name("deepl-language-picker");
        popover.set_constrain_to(gtk::PopoverConstraint::None);
        popover.set_pointing_to(&gtk::gdk::Rectangle::new(anchor.x.round() as i32, anchor.y.round() as i32, anchor.width.round() as i32, anchor.height.round() as i32));
        let bounds=view.window().and_then(|w|view.display().monitor_at_window(&w)).map(|m|m.workarea());
        let placement=bounds.map(|r|crate::core::popup::place_popup(
            Anchor{x:origin.0+anchor.x,y:origin.1+anchor.y,width:anchor.width,height:anchor.height},
            crate::core::placement::WorkArea{x:r.x(),y:r.y(),width:r.width().max(1) as u32,height:r.height().max(1) as u32},(280,450),6));
        let above=placement.is_some_and(|p|p.above);
        let content_height=placement.map(|p|(p.height as i32-96).clamp(40,360)).unwrap_or(240);
        popover.set_position(if above{gtk::PositionType::Top}else{gtk::PositionType::Bottom});
        #[cfg(debug_assertions)]
        eprintln!("Picker {revision}: KDE origin={origin:?}, above={above}, content_height={content_height}");
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 6);
        panel.set_border_width(8);panel.set_size_request(280, -1);
        let search = gtk::SearchEntry::new();search.set_placeholder_text(Some(&placeholder));
        let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);scroll.set_min_content_height(180.min(content_height));scroll.set_max_content_height(content_height);scroll.set_propagate_natural_height(true);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let mut buttons = Vec::new();
        for choice in choices {
            let button = gtk::Button::new();button.set_relief(gtk::ReliefStyle::None);
            let label = gtk::Label::new(Some(&format!("{}{}", if choice.code == selected { "✓  " } else { "    " }, choice.label)));label.set_xalign(0.);button.add(&label);
            let code = choice.code.clone();let app = owner.clone();
            button.connect_clicked(move |_| { if let Err(error) = language_popup::choose(&app, revision, code.clone()) { let _ = app.emit("notice", error); } });
            buttons.push((button.clone(), format!("{} {}",choice.label,choice.code).to_lowercase()));list.pack_start(&button,false,false,0);
        }
        let buttons = std::rc::Rc::new(buttons);
        let buttons_search = buttons.clone();
        let scroll_filter=scroll.clone();search.connect_search_changed(move |search| { let q=search.text().to_lowercase();let mut count=0;for (button,label) in buttons_search.iter() {let visible=label.contains(&q);button.set_visible(visible);if visible {count+=1;}}scroll_filter.set_min_content_height((count.clamp(1,10)*32).min(content_height)); });
        let buttons_enter=buttons.clone();search.connect_activate(move |_| { if let Some((button,_))=buttons_enter.iter().find(|(b,_)| b.is_visible()) {button.clicked();} });
        let buttons_down=buttons.clone();search.connect_key_press_event(move |_,event| {if event.keyval()==gtk::gdk::keys::constants::Down {if let Some((button,_))=buttons_down.iter().find(|(b,_)|b.is_visible()){button.grab_focus();}gtk::glib::Propagation::Stop}else{gtk::glib::Propagation::Proceed}});
        scroll.add(&list);panel.pack_start(&search,false,false,0);panel.pack_start(&scroll,true,true,0);popover.add(&panel);
        let (bg,ink,line,hover)=if dark {("#172532","#e0e9f0","#2a3a49","#293d4e")}else{("#ffffff","#17334b","#dce5eb","#e3ecef")};
        let css=gtk::CssProvider::new();let _=css.load_from_data(format!("#deepl-language-picker{{background:{bg};color:{ink};border:1px solid {line};}} #deepl-language-picker entry,#deepl-language-picker button{{background:{bg};color:{ink};font-size:13px;}} #deepl-language-picker button:hover{{background:{hover};}} ").as_bytes());
        let screen=view.screen();
        if let Some(screen)=&screen{gtk::StyleContext::add_provider_for_screen(screen,&css,gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);}
        let owner_closed=owner.clone();popover.connect_closed(move |_| {
            if let Some(screen)=&screen{gtk::StyleContext::remove_provider_for_screen(screen,&css);}
            language_popup::close_native_revision(&owner_closed,revision);
        });
        panel.reorder_child(&search,if above{1}else{0});
        POPOVER.with(|slot| *slot.borrow_mut()=Some(popover.clone()));popover.show_all();popover.popup();search.grab_focus();
    });
    if let Err(error) = opened {
        language_popup::abort_native_open(&app, revision);
        return Err(error.to_string());
    }
    Ok(revision)
}
