//! The Properties panel (top of a note, and the "File properties" sidebar
//! tab), the "All properties" pane and the property dialogs. Drawing only:
//! every change is emitted as an [`Action::Props`].

use std::collections::BTreeMap;

use chrono::{Datelike, Months, NaiveDate};
use eframe::egui::{
    self, Align, Color32, Frame, Id, Key, Layout, Margin, Popup, PopupCloseBehavior, RichText,
    Sense, Stroke, TextEdit, vec2,
};
use ors_core::NotePath;
use ors_vault::PropertyType;
use serde_json::Value;

use super::Action;
use super::theme::Palette;
use crate::properties_model::{
    self as model, MENU_TYPES, PanelContent, PropAction, PropRow, PropsModal, SortMode, Widget,
};
use crate::state::{AppState, Modal};

const RED: Color32 = Color32::from_rgb(0xe9, 0x31, 0x47);

fn emit(ui: &egui::Ui, actions: &mut Vec<Action>, a: PropAction) {
    actions.push(Action::Props(a));
    ui.ctx().request_repaint();
}

/// Property types, cached per index generation (reading `types.json` on
/// every frame would be wasteful).
fn cached_types(ctx: &egui::Context, state: &AppState) -> BTreeMap<String, PropertyType> {
    let id = Id::unique("props-types-cache");
    if let Some((generation, types)) =
        ctx.data(|d| d.get_temp::<(u64, BTreeMap<String, PropertyType>)>(id))
        && generation == state.generation
    {
        return types;
    }
    let types = state.property_types();
    ctx.data_mut(|d| d.insert_temp(id, (state.generation, types.clone())));
    types
}

// ---------------------------------------------------------------------------
// Small widgets
// ---------------------------------------------------------------------------

/// A pill with a text and a remove button. Returns `(text clicked, removed)`.
fn pill(ui: &mut egui::Ui, text: &str, id: Id, pal: &Palette) -> (bool, bool) {
    let font = egui::FontId::proportional(13.0);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font.clone(), pal.accent);
    let x_w = 14.0;
    let size = vec2(
        8.0 + galley.size().x + 4.0 + x_w + 2.0,
        galley.size().y + 4.0,
    );
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let x_rect = egui::Rect::from_min_size(
        egui::pos2(rect.right() - x_w - 2.0, rect.top()),
        vec2(x_w, rect.height()),
    );
    let text_rect = egui::Rect::from_min_max(rect.min, egui::pos2(x_rect.left(), rect.bottom()));
    let r_text = ui.interact(text_rect, id.with("text"), Sense::click());
    let r_x = ui.interact(x_rect, id.with("x"), Sense::click());
    let hovered = r_text.hovered() || r_x.hovered();
    let bg = if hovered { pal.accent_soft } else { pal.tag_bg };
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(10), bg);
    ui.painter().galley(
        egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0),
        galley,
        pal.accent,
    );
    let x_color = if r_x.hovered() { RED } else { pal.muted };
    ui.painter().text(
        x_rect.center(),
        egui::Align2::CENTER_CENTER,
        "×",
        egui::FontId::proportional(14.0),
        x_color,
    );
    (
        r_text
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked(),
        r_x.on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked(),
    )
}

/// A single-line field that edits a draft and returns the new text when
/// committed (Enter or focus lost). Escape discards.
fn draft_field(
    ui: &mut egui::Ui,
    id: Id,
    current: &str,
    width: f32,
    hint: &str,
    valid: impl Fn(&str) -> bool,
    request_focus: bool,
) -> Option<String> {
    let draft: Option<String> = ui.data(|d| d.get_temp(id));
    let mut text = draft.unwrap_or_else(|| current.to_string());
    let bad = !valid(&text);
    let resp = ui.add(
        TextEdit::singleline(&mut text)
            .id(id)
            .frame(Frame::NONE)
            .margin(Margin::symmetric(2, 1))
            .hint_text(RichText::new(hint).color(ui.visuals().weak_text_color()))
            .text_color_opt(bad.then_some(RED))
            .desired_width(width),
    );
    if request_focus {
        resp.request_focus();
    }
    if resp.has_focus() || resp.changed() {
        ui.data_mut(|d| d.insert_temp(id, text.clone()));
    }
    if resp.lost_focus() {
        let esc = ui.input(|i| i.key_pressed(Key::Escape));
        ui.data_mut(|d| d.remove::<String>(id));
        if !esc && text != current && valid(&text) {
            return Some(text);
        }
    }
    None
}

/// A small month calendar in a popup. Returns the picked date.
fn calendar_button(ui: &mut egui::Ui, id: Id, current: &str) -> Option<String> {
    let btn = ui.add(
        egui::Button::new(RichText::new("📅").size(12.0))
            .frame(false)
            .small(),
    );
    let sel = ors_vault::parse_date(current.trim());
    let today = chrono::Local::now().date_naive();
    let view_id = id.with("view");
    let mut picked = None;
    if btn.clicked() {
        let base = sel.unwrap_or(today).with_day(1).unwrap_or(today);
        ui.data_mut(|d| d.insert_temp(view_id, base));
    }
    Popup::from_toggle_button_response(&btn)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let mut view: NaiveDate = ui
                .data(|d| d.get_temp(view_id))
                .unwrap_or(sel.unwrap_or(today));
            ui.horizontal(|ui| {
                if ui.small_button("◀").clicked() {
                    view = view.checked_sub_months(Months::new(1)).unwrap_or(view);
                }
                ui.label(RichText::new(view.format("%B %Y").to_string()).strong());
                if ui.small_button("▶").clicked() {
                    view = view.checked_add_months(Months::new(1)).unwrap_or(view);
                }
            });
            egui::Grid::new(id.with("grid"))
                .spacing([2.0, 2.0])
                .show(ui, |ui| {
                    for d in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"] {
                        ui.label(RichText::new(d).small().weak());
                    }
                    ui.end_row();
                    let lead = view.weekday().num_days_from_monday() as usize;
                    for _ in 0..lead {
                        ui.label("");
                    }
                    let mut col = lead;
                    let mut day = view;
                    while day.month() == view.month() {
                        let is_sel = Some(day) == sel;
                        let text = RichText::new(day.day().to_string()).color(if day == today {
                            ui.visuals().hyperlink_color
                        } else {
                            ui.visuals().text_color()
                        });
                        if ui
                            .add(egui::Button::selectable(is_sel, text).small())
                            .clicked()
                        {
                            picked = Some(day.format("%Y-%m-%d").to_string());
                            ui.close();
                        }
                        col += 1;
                        if col == 7 {
                            ui.end_row();
                            col = 0;
                        }
                        let Some(next) = day.succ_opt() else { break };
                        day = next;
                    }
                });
            ui.data_mut(|d| d.insert_temp(view_id, view));
        });
    picked
}

// ---------------------------------------------------------------------------
// Value editors
// ---------------------------------------------------------------------------

struct RowCtx<'a> {
    state: &'a AppState,
    path: &'a NotePath,
    pal: Palette,
    scope: &'a str,
}

impl RowCtx<'_> {
    fn id(&self, key: &str, what: &str) -> Id {
        Id::unique((
            "prop",
            self.scope,
            self.path.as_str(),
            key.to_lowercase(),
            what,
        ))
    }
}

fn set_value(ui: &egui::Ui, actions: &mut Vec<Action>, cx: &RowCtx, key: &str, value: Value) {
    emit(
        ui,
        actions,
        PropAction::SetValue {
            path: cx.path.clone(),
            key: key.to_string(),
            value,
        },
    );
}

fn value_editor(
    ui: &mut egui::Ui,
    cx: &RowCtx,
    row: &PropRow,
    focus: bool,
    actions: &mut Vec<Action>,
) {
    let value = model::display_value(row.ty, &row.value);
    match model::widget_for(row.ty) {
        Widget::Text => {
            let cur = model::value_text(&value);
            let ty = row.ty;
            if let Some(t) = draft_field(
                ui,
                cx.id(&row.key, "text"),
                &cur,
                ui.available_width(),
                "Empty",
                |_| true,
                focus,
            ) && let Ok(v) = model::value_from_text(ty, &t)
            {
                set_value(ui, actions, cx, &row.key, v);
            }
        }
        Widget::Number => {
            let cur = model::value_text(&value);
            if let Some(t) = draft_field(
                ui,
                cx.id(&row.key, "num"),
                &cur,
                ui.available_width(),
                "Empty",
                |t| t.trim().is_empty() || model::value_from_text(PropertyType::Number, t).is_ok(),
                focus,
            ) && let Ok(v) = model::value_from_text(PropertyType::Number, &t)
            {
                set_value(ui, actions, cx, &row.key, v);
            }
        }
        Widget::Checkbox => {
            let mut b = model::checkbox_of(&value);
            if ui.checkbox(&mut b, "").changed() {
                set_value(ui, actions, cx, &row.key, Value::Bool(b));
            }
        }
        Widget::Date => {
            let cur = model::value_text(&value);
            ui.horizontal(|ui| {
                if let Some(t) = draft_field(
                    ui,
                    cx.id(&row.key, "date"),
                    &cur,
                    100.0,
                    "YYYY-MM-DD",
                    model::date_ok,
                    focus,
                ) {
                    let v = if t.trim().is_empty() {
                        Value::Null
                    } else {
                        Value::String(t.trim().to_string())
                    };
                    set_value(ui, actions, cx, &row.key, v);
                }
                if let Some(d) = calendar_button(ui, cx.id(&row.key, "cal"), &cur) {
                    set_value(ui, actions, cx, &row.key, Value::String(d));
                }
            });
        }
        Widget::DateTime => {
            let (d, t) = model::split_datetime(&model::value_text(&value));
            ui.horizontal(|ui| {
                let new_d = draft_field(
                    ui,
                    cx.id(&row.key, "dt-date"),
                    &d,
                    100.0,
                    "YYYY-MM-DD",
                    model::date_ok,
                    focus,
                );
                if let Some(nd) = calendar_button(ui, cx.id(&row.key, "cal"), &d)
                    && let Some(v) = model::join_datetime(&nd, &t)
                {
                    set_value(ui, actions, cx, &row.key, v);
                }
                let new_t = draft_field(
                    ui,
                    cx.id(&row.key, "dt-time"),
                    &t,
                    50.0,
                    "HH:mm",
                    model::time_ok,
                    false,
                );
                if new_d.is_some() || new_t.is_some() {
                    let nd = new_d.unwrap_or(d.clone());
                    let nt = new_t.unwrap_or(t.clone());
                    if let Some(v) = model::join_datetime(&nd, &nt) {
                        set_value(ui, actions, cx, &row.key, v);
                    }
                }
            });
        }
        Widget::List { tags } => list_editor(ui, cx, row, &value, tags, focus, actions),
    }
}

fn list_editor(
    ui: &mut egui::Ui,
    cx: &RowCtx,
    row: &PropRow,
    value: &Value,
    tags: bool,
    focus: bool,
    actions: &mut Vec<Action>,
) {
    let items = model::list_items(row.ty, value);
    let field_id = cx.id(&row.key, "add");
    let sel_id = cx.id(&row.key, "sel");
    let open_id = cx.id(&row.key, "popup");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 3.0);
        for (i, item) in items.iter().enumerate() {
            let label = if tags {
                format!("#{item}")
            } else {
                item.clone()
            };
            let (clicked, removed) =
                pill(ui, &label, cx.id(&row.key, &format!("pill{i}")), &cx.pal);
            if clicked && tags {
                actions.push(Action::SearchTag(item.clone()));
            }
            if removed {
                set_value(
                    ui,
                    actions,
                    cx,
                    &row.key,
                    model::list_without(row.ty, value, i),
                );
            }
        }
        let mut text: String = ui.data(|d| d.get_temp(field_id)).unwrap_or_default();
        let resp = ui.add(
            TextEdit::singleline(&mut text)
                .id(field_id)
                .frame(Frame::NONE)
                .margin(Margin::symmetric(2, 1))
                .hint_text(
                    RichText::new(if items.is_empty() { "Empty" } else { "" })
                        .color(ui.visuals().weak_text_color()),
                )
                .desired_width(if items.is_empty() { 80.0 } else { 60.0 }),
        );
        if focus {
            resp.request_focus();
        }
        // Suggestions.
        let popup_was_open: bool = ui.data(|d| d.get_temp(open_id)).unwrap_or(false);
        let mut sel: i32 = ui.data(|d| d.get_temp(sel_id)).unwrap_or(-1);
        let show_popup = resp.has_focus() || popup_was_open;
        let suggestions: Vec<(String, usize)> = if show_popup {
            suggestions_for(cx.state, &row.key, tags, &text, &items)
        } else {
            Vec::new()
        };
        if resp.has_focus() {
            ui.input(|i| {
                if i.key_pressed(Key::ArrowDown) {
                    sel = (sel + 1).min(suggestions.len() as i32 - 1);
                }
                if i.key_pressed(Key::ArrowUp) {
                    sel = (sel - 1).max(-1);
                }
            });
        }
        let mut add: Option<String> = None;
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        if resp.lost_focus() && enter {
            add = Some(
                match usize::try_from(sel).ok().and_then(|i| suggestions.get(i)) {
                    Some((s, _)) => s.clone(),
                    None => text.clone(),
                },
            );
            resp.request_focus();
        } else if resp.changed() && text.ends_with(',') {
            add = Some(text.clone());
        }
        // Popup.
        let mut popup_open = false;
        if show_popup && !suggestions.is_empty() {
            let area = egui::Area::new(cx.id(&row.key, "area"))
                .order(egui::Order::Foreground)
                .fixed_pos(resp.rect.left_bottom() + vec2(0.0, 2.0))
                .show(ui.ctx(), |ui| {
                    Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(170.0);
                        for (i, (name, count)) in suggestions.iter().enumerate() {
                            let label = if tags {
                                format!("#{name}")
                            } else {
                                name.clone()
                            };
                            let r = ui.horizontal(|ui| {
                                let r = ui.selectable_label(i as i32 == sel, label);
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(RichText::new(count.to_string()).small().weak());
                                });
                                r
                            });
                            if r.inner.clicked()
                                || (r.inner.hovered() && ui.input(|i| i.pointer.primary_pressed()))
                            {
                                add = Some(name.clone());
                            }
                        }
                    });
                });
            let hovered = ui
                .ctx()
                .pointer_hover_pos()
                .is_some_and(|p| area.response.rect.contains(p));
            popup_open = resp.has_focus() || hovered;
        }
        ui.data_mut(|d| d.insert_temp(open_id, popup_open));
        if let Some(input) = add {
            if let Some(v) = model::list_with_added(row.ty, value, &input) {
                set_value(ui, actions, cx, &row.key, v);
            }
            text.clear();
            sel = -1;
        }
        ui.data_mut(|d| {
            d.insert_temp(field_id, text.clone());
            d.insert_temp(sel_id, sel);
        });
        if !resp.has_focus() && !popup_open {
            ui.data_mut(|d| {
                d.remove::<String>(field_id);
            });
        }
    });
}

fn suggestions_for(
    state: &AppState,
    key: &str,
    tags: bool,
    typed: &str,
    items: &[String],
) -> Vec<(String, usize)> {
    let q = typed.trim().trim_start_matches('#').to_lowercase();
    let taken = |s: &str| items.iter().any(|i| i.eq_ignore_ascii_case(s));
    if tags {
        state
            .index
            .tag_suggestions(&q, 12)
            .into_iter()
            .filter(|(n, _)| !taken(n))
            .take(6)
            .collect()
    } else {
        let mut v: Vec<(String, usize)> = state
            .index
            .property_values(key)
            .into_iter()
            .filter(|(n, _)| !taken(n) && n.to_lowercase().contains(&q))
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v.truncate(6);
        v
    }
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

fn type_menu(
    ui: &mut egui::Ui,
    path: Option<&NotePath>,
    key: &str,
    current: PropertyType,
    actions: &mut Vec<Action>,
) {
    ui.label(RichText::new("Property type").small().weak());
    if model::is_builtin_key(key) {
        ui.add_enabled(
            false,
            egui::Button::new(format!(
                "{}  {} (built-in)",
                model::type_icon(current),
                model::type_label(current)
            )),
        );
        return;
    }
    for ty in MENU_TYPES {
        let mark = if ty == current { "✓" } else { "  " };
        if ui
            .button(format!(
                "{mark} {}  {}",
                model::type_icon(ty),
                model::type_label(ty)
            ))
            .clicked()
        {
            emit(
                ui,
                actions,
                PropAction::ChangeType {
                    path: path.cloned(),
                    key: key.to_string(),
                    ty,
                },
            );
            ui.close();
        }
    }
}

fn row_ui(
    ui: &mut egui::Ui,
    cx: &RowCtx,
    row: &PropRow,
    label_w: f32,
    focus: bool,
    actions: &mut Vec<Action>,
) {
    let value_w = (ui.available_width() - label_w - ui.spacing().item_spacing.x).max(60.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.allocate_ui_with_layout(
            vec2(label_w, 22.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                let r = ui.add(
                    egui::Label::new(
                        RichText::new(format!("{}  {}", model::type_icon(row.ty), row.key))
                            .color(cx.pal.muted),
                    )
                    .truncate()
                    .sense(Sense::click()),
                );
                let r = r
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(&row.key);
                Popup::menu(&r).show(|ui| {
                    ui.set_min_width(160.0);
                    type_menu(ui, Some(cx.path), &row.key, row.ty, actions);
                    ui.separator();
                    if ui.button("Rename…").clicked() {
                        actions.push(Action::Modal(Modal::Props(PropsModal::RenameProperty {
                            path: Some(cx.path.clone()),
                            key: row.key.clone(),
                            new: row.key.clone(),
                        })));
                        ui.close();
                    }
                    if ui.button("Search notes with this property").clicked() {
                        emit(ui, actions, PropAction::SearchProperty(row.key.clone()));
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .button(RichText::new("Remove property").color(RED))
                        .clicked()
                    {
                        emit(
                            ui,
                            actions,
                            PropAction::Remove {
                                path: cx.path.clone(),
                                key: row.key.clone(),
                            },
                        );
                        ui.close();
                    }
                });
            },
        );
        ui.allocate_ui_with_layout(
            vec2(value_w, 22.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_width(value_w);
                value_editor(ui, cx, row, focus, actions);
            },
        );
    });
}

#[derive(Clone, Default)]
struct AddState {
    open: bool,
    name: String,
    sel: i32,
}

fn add_row(
    ui: &mut egui::Ui,
    cx: &RowCtx,
    existing: &[String],
    types: &BTreeMap<String, PropertyType>,
    force_open: bool,
    actions: &mut Vec<Action>,
) {
    let id = Id::unique(("prop-add", cx.scope, cx.path.as_str()));
    let mut st: AddState = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    if force_open && !st.open {
        st = AddState {
            open: true,
            ..AddState::default()
        };
    }
    if !st.open {
        let r = ui.add(
            egui::Button::new(RichText::new("+  Add property").color(cx.pal.muted)).frame(false),
        );
        if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            st.open = true;
            st.name.clear();
            st.sel = -1;
        }
        ui.data_mut(|d| d.insert_temp(id, st));
        return;
    }
    let all = cx.state.index.properties();
    let suggestions = model::key_suggestions(&all, types, &st.name, existing, 8);
    let field_id = id.with("field");
    let resp = ui.add(
        TextEdit::singleline(&mut st.name)
            .id(field_id)
            .hint_text("Property name")
            .desired_width(180.0),
    );
    if force_open || (!resp.has_focus() && !resp.lost_focus() && st.sel == -1 && st.name.is_empty())
    {
        resp.request_focus();
    }
    let popup_open: bool = ui.data(|d| d.get_temp(id.with("popup"))).unwrap_or(false);
    // `has_focus` locks the context: never call it inside `ui.input`.
    let focused = resp.has_focus();
    ui.input(|i| {
        if focused && i.key_pressed(Key::ArrowDown) {
            st.sel = (st.sel + 1).min(suggestions.len() as i32 - 1);
        }
        if focused && i.key_pressed(Key::ArrowUp) {
            st.sel = (st.sel - 1).max(-1);
        }
    });
    let mut chosen: Option<(String, PropertyType)> = None;
    let enter = ui.input(|i| i.key_pressed(Key::Enter));
    let esc = ui.input(|i| i.key_pressed(Key::Escape));
    if resp.lost_focus() && enter {
        chosen = match usize::try_from(st.sel)
            .ok()
            .and_then(|i| suggestions.get(i))
        {
            Some((n, t)) => Some((n.clone(), *t)),
            None if !st.name.trim().is_empty() => {
                let name = st.name.trim().to_string();
                let ty = model::type_of_key(&name, types)
                    .or_else(|| {
                        all.get(&name.to_lowercase())
                            .map(|i| model::info_type(&i.name, i.inferred_type, types))
                    })
                    .unwrap_or(PropertyType::Text);
                Some((name, ty))
            }
            None => None,
        };
    }
    let mut now_open = false;
    if (resp.has_focus() || popup_open) && !suggestions.is_empty() {
        let area = egui::Area::new(id.with("area"))
            .order(egui::Order::Foreground)
            .fixed_pos(resp.rect.left_bottom() + vec2(0.0, 2.0))
            .show(ui.ctx(), |ui| {
                Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(180.0);
                    for (i, (name, ty)) in suggestions.iter().enumerate() {
                        let r = ui.selectable_label(
                            i as i32 == st.sel,
                            format!("{}  {}", model::type_icon(*ty), name),
                        );
                        if r.clicked() || (r.hovered() && ui.input(|i| i.pointer.primary_pressed()))
                        {
                            chosen = Some((name.clone(), *ty));
                        }
                    }
                });
            });
        let hovered = ui
            .ctx()
            .pointer_hover_pos()
            .is_some_and(|p| area.response.rect.contains(p));
        now_open = resp.has_focus() || hovered;
    }
    ui.data_mut(|d| d.insert_temp(id.with("popup"), now_open));
    if let Some((name, ty)) = chosen {
        emit(
            ui,
            actions,
            PropAction::Add {
                path: cx.path.clone(),
                key: name.clone(),
                ty,
            },
        );
        ui.data_mut(|d| d.insert_temp(Id::unique(("prop-focus", cx.path.as_str())), name));
        st = AddState::default();
    } else if esc || (resp.lost_focus() && !now_open) {
        st = AddState::default();
    }
    ui.data_mut(|d| d.insert_temp(id, st));
}

// ---------------------------------------------------------------------------
// The panel
// ---------------------------------------------------------------------------

/// The Properties panel of note `path`. `scope` distinguishes several panels
/// showing the same note; `document` is true in the note (collapsible title,
/// hidden without frontmatter) and false in the sidebar.
pub fn panel(
    ui: &mut egui::Ui,
    state: &AppState,
    path: &NotePath,
    scope: &str,
    document: bool,
    actions: &mut Vec<Action>,
) {
    if !path.is_markdown() {
        return;
    }
    let pal = Palette::of(ui.ctx());
    let source = state.note_text(path).unwrap_or_default();
    let types = cached_types(ui.ctx(), state);
    let content = model::panel_content(&source, &types);
    let force_add = state.add_property_requested;
    if document && content == PanelContent::Missing && !force_add {
        return;
    }
    if force_add {
        emit(ui, actions, PropAction::ClearAddRequest);
    }
    let cx = RowCtx {
        state,
        path,
        pal,
        scope,
    };
    let focus_id = Id::unique(("prop-focus", path.as_str()));
    let focus_key: Option<String> = ui.data(|d| d.get_temp(focus_id));
    let body = |ui: &mut egui::Ui, actions: &mut Vec<Action>| match &content {
        PanelContent::Invalid { yaml } => {
            Frame::new()
                .stroke(Stroke::new(1.0, RED))
                .corner_radius(6)
                .inner_margin(Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Invalid properties").strong().color(RED));
                    ui.label(
                        RichText::new("The YAML cannot be parsed. Fix it in the source.")
                            .small()
                            .color(pal.muted),
                    );
                    ui.add_space(4.0);
                    ui.label(RichText::new(yaml).monospace().size(12.0).color(pal.text));
                });
        }
        PanelContent::Missing | PanelContent::Rows(_) => {
            let rows: &[PropRow] = match &content {
                PanelContent::Rows(r) => r,
                _ => &[],
            };
            let label_w = (ui.available_width() * 0.3).clamp(90.0, 190.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            for row in rows {
                let focus = focus_key
                    .as_deref()
                    .is_some_and(|k| k.eq_ignore_ascii_case(&row.key));
                row_ui(ui, &cx, row, label_w, focus, actions);
            }
            if focus_key.is_some() {
                ui.data_mut(|d| d.remove::<String>(focus_id));
            }
            let existing: Vec<String> = rows.iter().map(|r| r.key.clone()).collect();
            add_row(ui, &cx, &existing, &types, force_add, actions);
        }
    };
    if document {
        let id = Id::unique(("props-collapse", scope));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| {
                ui.label(RichText::new("Properties").strong().color(pal.muted));
            })
            .body(|ui| body(ui, actions));
        ui.add_space(6.0);
    } else {
        body(ui, actions);
    }
}

// ---------------------------------------------------------------------------
// "All properties" pane
// ---------------------------------------------------------------------------

pub fn all_properties_pane(ui: &mut egui::Ui, state: &AppState, actions: &mut Vec<Action>) {
    let pal = Palette::of(ui.ctx());
    let sort_id = Id::unique("all-props-sort");
    let mut sort: SortMode = ui.data(|d| d.get_temp(sort_id)).unwrap_or_default();
    let all = state.index.properties();
    let types = cached_types(ui.ctx(), state);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("All properties ({})", all.len()))
                .small()
                .strong()
                .color(pal.muted),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let r = ui
                .add(egui::Button::new("⇅").frame(false))
                .on_hover_text("Sort");
            Popup::menu(&r).show(|ui| {
                for m in SortMode::ALL {
                    if ui.selectable_label(sort == m, m.label()).clicked() {
                        sort = m;
                        ui.close();
                    }
                }
            });
        });
    });
    ui.data_mut(|d| d.insert_temp(sort_id, sort));
    if all.is_empty() {
        ui.label(RichText::new("No properties found.").color(pal.muted));
        return;
    }
    let mut list: Vec<_> = all.values().collect();
    list.sort_by(|a, b| sort.compare((&a.name, a.count), (&b.name, b.count)));
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for info in list {
                let ty = model::info_type(&info.name, info.inferred_type, &types);
                let r = ui
                    .horizontal(|ui| {
                        let r = ui.add(
                            egui::Label::new(
                                RichText::new(format!("{}  {}", model::type_icon(ty), info.name))
                                    .color(pal.text),
                            )
                            .truncate()
                            .sense(Sense::click()),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(info.count.to_string())
                                    .small()
                                    .color(pal.faint),
                            );
                        });
                        r
                    })
                    .inner;
                let r = r
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(format!("{} ({})", model::type_label(ty), info.count));
                if r.clicked() {
                    emit(ui, actions, PropAction::SearchProperty(info.name.clone()));
                }
                r.context_menu(|ui| {
                    type_menu(ui, None, &info.name, ty, actions);
                    ui.separator();
                    if ui.button("Rename everywhere…").clicked() {
                        actions.push(Action::Modal(Modal::Props(PropsModal::RenameProperty {
                            path: None,
                            key: info.name.clone(),
                            new: info.name.clone(),
                        })));
                        ui.close();
                    }
                });
            }
        });
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------

/// Draw a properties / tags dialog. Returns the dialog to keep open.
pub fn dialog(
    ctx: &egui::Context,
    state: &AppState,
    modal: PropsModal,
    actions: &mut Vec<Action>,
) -> Option<Modal> {
    match modal {
        PropsModal::RenameTag { old, mut new } => {
            let new_clean = ors_markdown::properties::normalize_tag(&new);
            let plan = cached_plan(ctx, state, &old, &new_clean);
            let merge =
                new_clean != old && state.index.tags().contains_key(&new_clean.to_lowercase());
            let mut confirm = false;
            let mut cancel = false;
            let resp = egui::Modal::new(Id::unique("props-modal")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(
                    RichText::new(format!("Rename tag #{old}"))
                        .strong()
                        .size(15.0),
                );
                ui.add_space(6.0);
                let te = ui.add(TextEdit::singleline(&mut new).desired_width(f32::INFINITY));
                if !te.has_focus() && !te.lost_focus() {
                    te.request_focus();
                }
                ui.add_space(4.0);
                let ok = match &plan {
                    Ok(n) if new_clean != old && !new_clean.is_empty() => {
                        ui.label(format!(
                            "{n} note{} will be modified.",
                            if *n == 1 { "" } else { "s" }
                        ));
                        if merge {
                            ui.label(
                                RichText::new(format!(
                                    "#{new_clean} already exists: tags are merged."
                                ))
                                .small()
                                .weak(),
                            );
                        }
                        ui.label(RichText::new("Nested tags follow.").small().weak());
                        true
                    }
                    Ok(_) => false,
                    Err(msg) => {
                        ui.colored_label(RED, msg);
                        false
                    }
                };
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(ok, egui::Button::new("Rename")).clicked()
                        || (ok && ui.input(|i| i.key_pressed(Key::Enter)))
                    {
                        confirm = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
            if confirm {
                actions.push(Action::Confirm(Modal::Props(PropsModal::RenameTag {
                    old,
                    new: new_clean,
                })));
                None
            } else if cancel || resp.should_close() {
                None
            } else {
                Some(Modal::Props(PropsModal::RenameTag { old, new }))
            }
        }
        PropsModal::RenameProperty { path, key, mut new } => {
            let count = state.index.files_with_property(&key).len();
            let mut here = false;
            let mut everywhere = false;
            let mut cancel = false;
            let resp = egui::Modal::new(Id::unique("props-modal")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(
                    RichText::new(format!("Rename property {key}"))
                        .strong()
                        .size(15.0),
                );
                ui.add_space(6.0);
                let te = ui.add(TextEdit::singleline(&mut new).desired_width(f32::INFINITY));
                if !te.has_focus() && !te.lost_focus() {
                    te.request_focus();
                }
                let ok = !new.trim().is_empty() && new.trim() != key;
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "{count} note{} use this property.",
                        if count == 1 { "" } else { "s" }
                    ))
                    .small()
                    .weak(),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if path.is_some()
                        && ui
                            .add_enabled(ok, egui::Button::new("Rename in this note"))
                            .clicked()
                    {
                        here = true;
                    }
                    if ui
                        .add_enabled(ok, egui::Button::new(format!("Rename in {count} notes")))
                        .clicked()
                    {
                        everywhere = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
            if here {
                actions.push(Action::Confirm(Modal::Props(PropsModal::RenameProperty {
                    path,
                    key,
                    new,
                })));
                None
            } else if everywhere {
                actions.push(Action::Confirm(Modal::Props(PropsModal::RenameProperty {
                    path: None,
                    key,
                    new,
                })));
                None
            } else if cancel || resp.should_close() {
                None
            } else {
                Some(Modal::Props(PropsModal::RenameProperty { path, key, new }))
            }
        }
    }
}

/// `plan_tag_rename` count, cached per (names, generation).
fn cached_plan(
    ctx: &egui::Context,
    state: &AppState,
    old: &str,
    new: &str,
) -> Result<usize, String> {
    let id = Id::unique("props-plan-cache");
    let key = (old.to_string(), new.to_string(), state.generation);
    if let Some((k, v)) =
        ctx.data(|d| d.get_temp::<((String, String, u64), Result<usize, String>)>(id))
        && k == key
    {
        return v;
    }
    let v = if new.is_empty() || new == old {
        Ok(0)
    } else {
        state
            .plan_tag_rename(old, new)
            .map(|p| p.len())
            .map_err(|e| e.to_string())
    };
    ctx.data_mut(|d| d.insert_temp(id, (key, v.clone())));
    v
}
