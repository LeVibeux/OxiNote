//! Daily notes and templates folder.

use chrono::{Local, NaiveDateTime};
use oxi_core::{Error, NotePath, Result, VaultRead, VaultWrite};

use crate::moment::expand_with;
use crate::{
    DEFAULT_DATE_FORMAT, DEFAULT_TIME_FORMAT, DailyNotesConfig, Date, TemplateContext,
    TemplatesConfig, Vault, format_moment,
};

/// Vault path of the daily note for `date`: `<folder>/<format(date)>.md`.
/// The format may contain `/` to put notes in subfolders.
pub fn daily_note_path(config: &DailyNotesConfig, date: Date) -> Result<NotePath> {
    let fmt = if config.format.trim().is_empty() {
        DEFAULT_DATE_FORMAT
    } else {
        config.format.as_str()
    };
    let name = format_moment(fmt, &TemplateContext::on_date("", date));
    let name = name.trim().trim_matches('/');
    if name.is_empty() {
        return Err(Error::InvalidPath(format!(
            "daily note format {fmt:?} yields an empty name"
        )));
    }
    NotePath::join(&config.folder, &format!("{name}.md"))
}

/// Expand a daily-note template like Obsidian's Daily notes plugin:
/// `{{title}}`, `{{date}}` (daily note format) / `{{time}}`,
/// `{{date:FMT}}` / `{{time:FMT}}`, `{{yesterday}}`, `{{tomorrow}}`.
pub(crate) fn expand_daily_template(
    template: &str,
    title: &str,
    date: Date,
    now: NaiveDateTime,
    daily: &DailyNotesConfig,
    templates: &TemplatesConfig,
) -> String {
    let mut ctx = TemplateContext::from_datetime(title, now);
    ctx.date = date;
    let date_fmt = if daily.format.trim().is_empty() {
        DEFAULT_DATE_FORMAT
    } else {
        daily.format.as_str()
    };
    let time_fmt = templates
        .time_format
        .as_deref()
        .unwrap_or(DEFAULT_TIME_FORMAT);
    let shifted = |days: i64| {
        let mut c = ctx.clone();
        c.date = date.add_days(days);
        format_moment(date_fmt, &c)
    };
    expand_with(template, |inner| {
        let (name, fmt) = match inner.split_once(':') {
            Some((n, f)) => (n.trim(), Some(f.trim())),
            None => (inner, None),
        };
        match (name.to_ascii_lowercase().as_str(), fmt) {
            ("title", None) => Some(ctx.title.clone()),
            ("date", None) => Some(format_moment(date_fmt, &ctx)),
            ("time", None) => Some(format_moment(time_fmt, &ctx)),
            ("date" | "time", Some(f)) => Some(format_moment(f, &ctx)),
            ("yesterday", None) => Some(shifted(-1)),
            ("tomorrow", None) => Some(shifted(1)),
            _ => None,
        }
    })
}

impl Vault {
    /// Vault path of the daily note for `date` according to
    /// `daily-notes.json` (the note is not created).
    pub fn daily_note_path(&self, date: Date) -> Result<NotePath> {
        daily_note_path(&self.daily_notes_config()?, date)
    }

    /// [`Vault::open_daily_note`] for [`Date::today`].
    pub fn open_today_note(&self) -> Result<NotePath> {
        self.open_daily_note(Date::today())
    }

    pub(crate) fn open_daily_note_impl(&self, date: Date) -> Result<NotePath> {
        if date.to_naive().is_none() {
            return Err(Error::InvalidPath(format!("invalid date {date}")));
        }
        let daily = self.daily_notes_config()?;
        let path = daily_note_path(&daily, date)?;
        if self.exists(&path) {
            return Ok(path);
        }
        let contents = match self.find_template(daily.template.as_deref())? {
            Some(template) => {
                let raw = self.read_to_string(&template)?;
                let templates = self.templates_config()?;
                expand_daily_template(
                    &raw,
                    path.stem(),
                    date,
                    Local::now().naive_local(),
                    &daily,
                    &templates,
                )
            }
            None => String::new(),
        };
        self.write(&path, &contents)?;
        Ok(path)
    }

    /// Resolve a template reference (`"Templates/Daily"` or with `.md`).
    fn find_template(&self, template: Option<&str>) -> Result<Option<NotePath>> {
        let Some(t) = template.map(str::trim).filter(|t| !t.is_empty()) else {
            return Ok(None);
        };
        let with_md = if t.to_ascii_lowercase().ends_with(".md") {
            t.to_string()
        } else {
            format!("{t}.md")
        };
        for candidate in [with_md.as_str(), t] {
            if let Ok(p) = NotePath::new(candidate)
                && self.exists(&p)
            {
                return Ok(Some(p));
            }
        }
        Ok(None)
    }

    pub(crate) fn list_templates_impl(&self) -> Result<Vec<NotePath>> {
        let Some(folder) = self.templates_config()?.folder else {
            return Ok(Vec::new());
        };
        let (norm, abs) = self.resolve_folder(&folder)?;
        if norm.is_empty() || !abs.is_dir() {
            // An unset or missing folder lists nothing (never the whole vault).
            return Ok(Vec::new());
        }
        let (files, _) = self.walk_from(&abs, &norm)?;
        let mut notes: Vec<NotePath> = files.into_iter().filter(NotePath::is_markdown).collect();
        notes.sort();
        Ok(notes)
    }

    /// Read template `template` and expand it for a new note titled `title`
    /// at the current local time, using `templates.json` formats.
    pub fn render_template(&self, template: &NotePath, title: &str) -> Result<String> {
        let raw = self.read_to_string(template)?;
        let config = self.templates_config()?;
        Ok(crate::expand_template(
            &raw,
            &TemplateContext::now(title),
            &config,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        let mut c = DailyNotesConfig::default();
        let d = Date::new(2026, 10, 20).unwrap();
        assert_eq!(daily_note_path(&c, d).unwrap().as_str(), "2026-10-20.md");
        c.folder = "Journal".into();
        c.format = "YYYY/MM/YYYY-MM-DD dddd".into();
        assert_eq!(
            daily_note_path(&c, d).unwrap().as_str(),
            "Journal/2026/10/2026-10-20 Tuesday.md"
        );
        c.format = "[]".into();
        assert!(daily_note_path(&c, d).is_err());
    }

    #[test]
    fn daily_template() {
        let daily = DailyNotesConfig {
            format: "DD.MM.YYYY".into(),
            folder: String::new(),
            template: None,
        };
        let now = chrono::NaiveDate::from_ymd_opt(2030, 1, 1)
            .unwrap()
            .and_hms_opt(9, 7, 0)
            .unwrap();
        let out = expand_daily_template(
            "{{title}}|{{date}}|{{time}}|{{yesterday}}|{{tomorrow}}|{{date:YYYY}}",
            "20.10.2026",
            Date::new(2026, 10, 20).unwrap(),
            now,
            &daily,
            &TemplatesConfig::default(),
        );
        assert_eq!(
            out,
            "20.10.2026|20.10.2026|09:07|19.10.2026|21.10.2026|2026"
        );
    }
}
