//! `0ass-gui` — interface graphique de revue.
//!
//! L'interface ne modifie jamais le projet sans validation humaine explicite :
//! chaque ligne de la table est un **candidat**, et c'est l'humain qui décide.
//!
//! Boutons disponibles :
//!
//! * **Scanner** : lance une campagne (scan → mesure → rapport) ;
//! * **Approuver** : worktree isolé, vérification de compatibilité, mesure A/B,
//!   un commit par patch ;
//! * **Rejeter** : le candidat sort de la file ;
//! * **Merge fast-forward dans main** : fusionne la branche `0ass/<date>`.
//!
//! Le `git push` n'existe pas ici : il reste à la main de l'utilisateur.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};

use eframe::egui;
use zeroass_core::manifest::{Manifest, ManifestProject, Status, TargetInfo};
use zeroass_core::profile::ProjectProfile;
use zeroass_engine::campaign::{self, CampaignOptions};
use zeroass_engine::review;

/// Résultat d'une tâche exécutée en arrière-plan.
struct TaskResult {
    label: String,
    message: String,
    manifest: Option<Manifest>,
    assembly: Option<AssemblyView>,
}

/// Assembleur affiché dans une fenêtre dédiée.
#[derive(Debug, Clone)]
struct AssemblyView {
    symbol: String,
    text: String,
    file: Option<String>,
    missing: bool,
    unoptimized: bool,
}

/// Une action demandée par l'utilisateur.
enum Task {
    Scan,
    Approve { id: String, patch: PathBuf },
    Measure { id: String, patch: PathBuf },
    Assembly { fn_name: String },
    Merge,
}

struct App {
    project: PathBuf,
    report_root: PathBuf,
    report_dir: Option<PathBuf>,
    manifest: Manifest,
    selected: Option<usize>,
    filter: String,
    only_pending: bool,
    status: String,
    busy: Option<String>,
    patch_path: String,
    picker: Option<Picker>,
    assembly: Option<AssemblyView>,
    backup_notice: Option<String>,
    tx: Sender<TaskResult>,
    rx: Receiver<TaskResult>,
}

/// Navigateur de dossiers : permet de changer de projet sans quitter l'interface.
struct Picker {
    /// Répertoire affiché.
    path: PathBuf,
    /// Sous-dossiers du répertoire affiché.
    entries: Vec<PathBuf>,
    /// Lecteurs (Windows) ou racine (Unix).
    volumes: Vec<PathBuf>,
    /// Chemin saisi à la main.
    typed: String,
    /// Message d'erreur éventuel.
    error: Option<String>,
}

impl Picker {
    fn new(path: PathBuf) -> Picker {
        let mut p = Picker {
            entries: Vec::new(),
            volumes: volumes(),
            typed: path.to_string_lossy().to_string(),
            error: None,
            path,
        };
        p.refresh();
        p
    }

    /// Relit le contenu du répertoire courant.
    fn refresh(&mut self) {
        self.error = None;
        self.typed = self.path.to_string_lossy().to_string();
        match std::fs::read_dir(&self.path) {
            Ok(rd) => {
                let mut dirs: Vec<PathBuf> = rd
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect();
                dirs.sort();
                self.entries = dirs;
            }
            Err(e) => {
                self.entries.clear();
                self.error = Some(format!("{} : {e}", self.path.display()));
            }
        }
    }

    /// Descend dans un sous-dossier.
    fn enter(&mut self, path: PathBuf) {
        self.path = path;
        self.refresh();
    }

    /// Remonte au dossier parent.
    fn up(&mut self) {
        if let Some(parent) = self.path.parent() {
            let parent = parent.to_path_buf();
            self.path = parent;
            self.refresh();
        }
    }
}

/// Liste les lecteurs (Windows) ou la racine (Unix).
fn volumes() -> Vec<PathBuf> {
    if cfg!(windows) {
        let mut out = Vec::new();
        for letter in b'A'..=b'Z' {
            let path = format!("{}:\\", letter as char);
            let p = PathBuf::from(path);
            if p.exists() {
                out.push(p);
            }
        }
        out
    } else {
        vec![PathBuf::from("/")]
    }
}

/// Manifeste vide (projet encore inconnu).
fn empty_manifest(root: &Path) -> Manifest {
    Manifest::new(
        ManifestProject {
            root: root.to_string_lossy().to_string(),
            name: root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "projet".into()),
            kind: String::new(),
            profile: None,
        },
        TargetInfo {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            rustc: None,
            profile: "release".into(),
        },
    )
}

impl App {
    fn new(project: PathBuf) -> App {
        let (tx, rx) = std::sync::mpsc::channel();
        let report_root = campaign::default_report_root(&project);
        let manifest = empty_manifest(&project);
        let mut app = App {
            project,
            report_root,
            report_dir: None,
            manifest,
            selected: None,
            filter: String::new(),
            only_pending: true,
            status: "prêt".into(),
            busy: None,
            patch_path: String::new(),
            picker: None,
            assembly: None,
            backup_notice: None,
            tx,
            rx,
        };
        app.reload();
        app
    }

    /// Change de projet : nouveau dossier de rapport, nouveau manifeste.
    fn set_project(&mut self, path: PathBuf) {
        if !path.is_dir() {
            self.status = format!("{} n'est pas un dossier", path.display());
            return;
        }
        self.project = path;
        self.report_root = campaign::default_report_root(&self.project);
        self.report_dir = None;
        self.manifest = empty_manifest(&self.project);
        self.selected = None;
        self.filter.clear();
        self.patch_path.clear();
        self.reload();
        self.status = format!("projet : {}", self.project.display());
    }

    /// Ouvre le navigateur de dossiers.
    fn open_picker(&mut self) {
        let start = if self.project.is_dir() {
            self.project.clone()
        } else {
            self.project
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
        };
        self.picker = Some(Picker::new(start));
    }

    /// Recharge le dernier rapport de la campagne.
    fn reload(&mut self) {
        match campaign::latest_report(&self.report_root) {
            Ok(dir) => match campaign::load_manifest(&dir) {
                Ok(m) => {
                    self.status = format!("rapport : {}", dir.display());
                    self.report_dir = Some(dir);
                    self.manifest = m;
                    self.sync_patch_path();
                }
                Err(e) => self.status = format!("manifeste illisible : {e}"),
            },
            Err(e) => {
                self.report_dir = None;
                self.status = format!("aucun rapport : {e}");
            }
        }
    }

    /// Propose le patch du candidat sélectionné.
    fn sync_patch_path(&mut self) {
        if let (Some(dir), Some(idx)) = (self.report_dir.clone(), self.selected) {
            if let Some(c) = self.manifest.candidates.get(idx) {
                if let Some(p) = &c.patch {
                    self.patch_path = p.clone();
                    return;
                }
                let guess = dir.join("patches").join(format!("{}.patch", c.id));
                if guess.exists() {
                    self.patch_path = guess.to_string_lossy().to_string();
                }
            }
        }
    }

    /// Indices des candidats affichés (après filtrage).
    fn visible(&self) -> Vec<usize> {
        let needle = self.filter.to_lowercase();
        self.manifest
            .candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                if self.only_pending && c.status != Status::Review {
                    return false;
                }
                if needle.is_empty() {
                    return true;
                }
                let hay = format!(
                    "{} {} {} {} {}",
                    c.id, c.fn_name, c.file, c.strategy, c.fn_name
                )
                .to_lowercase();
                hay.contains(&needle)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Lance une tâche en arrière-plan (l'interface ne se fige pas).
    fn spawn(&mut self, task: Task, label: &str) {
        if self.busy.is_some() {
            self.status = "une tâche est déjà en cours".into();
            return;
        }
        let project = self.project.clone();
        let report_root = self.report_root.clone();
        let report_dir = self.report_dir.clone();
        let tx = self.tx.clone();
        let label = label.to_string();
        self.busy = Some(label.clone());
        self.status = format!("{label}…");
        std::thread::spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match task {
                Task::Scan => {
                    let options = CampaignOptions {
                        project_root: project.clone(),
                        report_root: Some(report_root.clone()),
                        skip_bench: false,
                    };
                    match campaign::run(&options) {
                        Ok(campaign) => TaskResult {
                            label: label.to_string(),
                            message: format!(
                                "{} candidat(s) — rapport : {}",
                                campaign.manifest.candidates.len(),
                                campaign.report_dir.display()
                            ),
                            manifest: Some(campaign.manifest),
                            assembly: None,
                        },
                        Err(e) => TaskResult {
                            label: label.to_string(),
                            message: format!("échec : {e}"),
                            manifest: None,
                            assembly: None,
                        },
                    }
                }
                Task::Approve { id, patch } => match report_dir {
                    None => TaskResult {
                        label: label.to_string(),
                        message: "aucun rapport".into(),
                        manifest: None,
                        assembly: None,
                    },
                    Some(dir) => match review::approve(&project, &dir, &id, &patch, None, None) {
                        Ok(outcome) => TaskResult {
                            label: label.to_string(),
                            message: outcome.message.clone(),
                            manifest: campaign::load_manifest(&dir).ok(),
                            assembly: None,
                        },
                        Err(e) => TaskResult {
                            label: label.to_string(),
                            message: format!("échec : {e}"),
                            manifest: None,
                            assembly: None,
                        },
                    },
                },
                Task::Measure { id, patch } => match report_dir {
                    None => TaskResult {
                        label: label.to_string(),
                        message: "aucun rapport".into(),
                        manifest: None,
                        assembly: None,
                    },
                    Some(dir) => match review::measure_patch(&project, &dir, &id, &patch, None) {
                        Ok(outcome) => TaskResult {
                            label: label.to_string(),
                            message: outcome.message.clone(),
                            manifest: campaign::load_manifest(&dir).ok(),
                            assembly: None,
                        },
                        Err(e) => TaskResult {
                            label: label.to_string(),
                            message: format!("échec : {e}"),
                            manifest: None,
                            assembly: None,
                        },
                    },
                },
                Task::Assembly { fn_name } => {
                    let profile = ProjectProfile::load_or_default(&project);
                    match zeroass_engine::asmview::emit_assembly(&profile, &project, &fn_name) {
                        Ok(res) => TaskResult {
                            label: label.to_string(),
                            message: if res.missing {
                                format!("{fn_name} : introuvable dans l'assembleur généré")
                            } else {
                                format!(
                                    "{fn_name} : {} lignes d'assembleur",
                                    res.assembly.lines().count()
                                )
                            },
                            manifest: None,
                            assembly: Some(AssemblyView {
                                symbol: res.symbol,
                                text: res.assembly,
                                file: res.file.map(|f| f.to_string_lossy().to_string()),
                                missing: res.missing,
                                unoptimized: res.unoptimized,
                            }),
                        },
                        Err(e) => TaskResult {
                            label: label.to_string(),
                            message: format!("échec : {e}"),
                            manifest: None,
                            assembly: None,
                        },
                    }
                }
                Task::Merge => match report_dir {
                    None => TaskResult {
                        label: label.to_string(),
                        message: "aucun rapport".into(),
                        manifest: None,
                        assembly: None,
                    },
                    Some(dir) => match review::merge_ff(&project, &dir) {
                        Ok(msg) => TaskResult {
                            label: label.to_string(),
                            message: msg,
                            manifest: None,
                            assembly: None,
                        },
                        Err(e) => TaskResult {
                            label: label.to_string(),
                            message: format!("échec : {e}"),
                            manifest: None,
                            assembly: None,
                        },
                    },
                },
            }));
            let result = match outcome {
                Ok(result) => result,
                Err(panic) => TaskResult {
                    label: label.clone(),
                    message: format!(
                        "la tâche a échoué : {}",
                        panic
                            .downcast_ref::<&str>()
                            .map(|s| (*s).to_string())
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "panique inconnue".into())
                    ),
                    manifest: None,
                    assembly: None,
                },
            };
            let _ = tx.send(result);
        });
    }
}

/// Libellé court d'un statut.
fn status_label(status: Status) -> &'static str {
    match status {
        Status::Review => "à revoir",
        Status::Approved => "approuvé",
        Status::Applied => "appliqué",
        Status::Rejected => "rejeté",
        Status::Stale => "périmé",
    }
}

/// Couleur d'un statut.
fn status_color(status: Status) -> egui::Color32 {
    match status {
        Status::Review => egui::Color32::from_rgb(230, 200, 80),
        Status::Approved => egui::Color32::from_rgb(110, 210, 130),
        Status::Applied => egui::Color32::from_rgb(110, 210, 130),
        Status::Rejected => egui::Color32::from_rgb(230, 110, 110),
        Status::Stale => egui::Color32::from_rgb(160, 160, 160),
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 1. Récupération des résultats des tâches.
        while let Ok(result) = self.rx.try_recv() {
            self.busy = None;
            self.status = format!("{} : {}", result.label, result.message);
            if let Some(a) = result.assembly {
                self.assembly = Some(a);
            }
            if let Some(m) = result.manifest {
                self.manifest = m;
                self.reload();
            }
        }
        if self.busy.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
        }

        // 2. Barre supérieure.
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("0ass");
                ui.separator();
                ui.label("projet :");
                let mut project_text = self.project.to_string_lossy().to_string();
                let response = ui.add(
                    egui::TextEdit::singleline(&mut project_text)
                        .desired_width(320.0)
                        .hint_text("chemin du projet"),
                );
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let candidate = PathBuf::from(project_text.trim());
                    if candidate.is_dir() {
                        self.set_project(candidate);
                    } else {
                        self.status = format!("{} n'est pas un dossier", candidate.display());
                    }
                }
                if ui
                    .button("…")
                    .on_hover_text("Changer de projet (navigateur de dossiers)")
                    .clicked()
                {
                    self.open_picker();
                }
                ui.separator();
                let busy = self.busy.is_some();
                if ui
                    .add_enabled(!busy, egui::Button::new("Scanner"))
                    .on_hover_text("Lance une campagne : scan → mesure → rapport")
                    .clicked()
                {
                    self.spawn(Task::Scan, "scan");
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Recharger"))
                    .on_hover_text("Relit le dernier rapport")
                    .clicked()
                {
                    self.reload();
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Merge fast-forward dans main"))
                    .on_hover_text("Fusionne la branche 0ass/<date> dans la branche courante.\nLe git push reste à ta main.")
                    .clicked()
                {
                    self.spawn(Task::Merge, "fusion");
                }
                if ui.button("Rapport (.md)").on_hover_text("Ouvre le dossier du rapport").clicked() {
                    if let Some(dir) = &self.report_dir {
                        open_path(dir);
                    }
                }
            });
        });

        // 3. Barre inférieure : état.
        egui::TopBottomPanel::bottom("bottom").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(busy) = &self.busy {
                    ui.spinner();
                    ui.label(busy);
                } else {
                    ui.label(&self.status);
                }
            });
        });

        // 4. Panneau latéral : filtres et liste.
        egui::SidePanel::left("liste")
            .resizable(true)
            .default_width(430.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Filtre").strong());
                    ui.text_edit_singleline(&mut self.filter);
                });
                ui.checkbox(&mut self.only_pending, "seulement les candidats à revoir");
                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    let visible = self.visible();
                    if visible.is_empty() {
                        ui.label("aucun candidat à afficher");
                    }
                    for idx in visible {
                        let c = self.manifest.candidates[idx].clone();
                        let selected = self.selected == Some(idx);
                        let title =
                            format!("{}  {}  ·  {}:{}", c.id, c.fn_name, c.file, c.start_line);
                        let response = ui
                            .horizontal(|ui| {
                                let resp = ui.selectable_label(selected, title);
                                let gain = match &c.measurement {
                                    Some(m) => format!("{:+.1} %", m.local_gain_pct),
                                    None => "—".to_string(),
                                };
                                ui.colored_label(status_color(c.status), status_label(c.status));
                                ui.colored_label(egui::Color32::GRAY, gain);
                                resp
                            })
                            .inner;
                        if response.clicked() {
                            self.selected = Some(idx);
                            self.sync_patch_path();
                        }
                        if response.double_clicked() {
                            if let Some(dir) = &self.report_dir {
                                let path = dir.join(&c.file);
                                open_path(&path);
                            }
                        }
                    }
                });
            });

        // 5. Navigateur de projet (fenêtre modale).
        if let Some(mut picker) = self.picker.take() {
            let mut open = true;
            let mut cancelled = false;
            let mut chosen: Option<PathBuf> = None;
            egui::Window::new("Changer de projet")
                .collapsible(false)
                .resizable(true)
                .default_width(560.0)
                .default_height(520.0)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("chemin :");
                        ui.text_edit_singleline(&mut picker.typed);
                    });
                    ui.horizontal(|ui| {
                        let typed = picker.typed.trim().to_string();
                        if ui
                            .button("Ouvrir ce dossier")
                            .on_hover_text("Charge ce projet et son dernier rapport")
                            .clicked()
                        {
                            let candidate = PathBuf::from(&typed);
                            if candidate.is_dir() {
                                chosen = Some(candidate);
                            } else {
                                picker.error = Some(format!("{typed} n'est pas un dossier"));
                            }
                        }
                        if ui.button("Annuler").clicked() {
                            cancelled = true;
                        }
                    });
                    if let Some(err) = &picker.error {
                        ui.colored_label(egui::Color32::from_rgb(230, 110, 110), err);
                    }
                    ui.separator();

                    ui.horizontal(|ui| {
                        if ui.button(".. (dossier parent)").clicked() {
                            picker.up();
                        }
                        for vol in picker.volumes.clone() {
                            let label = vol.to_string_lossy().to_string();
                            if ui.button(label.clone()).clicked() {
                                picker.enter(vol);
                            }
                        }
                    });
                    ui.separator();
                    ui.label(format!("dossiers de {}", picker.path.display()));
                    ui.separator();

                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if picker.entries.is_empty() {
                            ui.label("(aucun sous-dossier)");
                        }
                        for entry in picker.entries.clone() {
                            let name = entry
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| entry.to_string_lossy().to_string());
                            if ui.selectable_label(false, name).clicked() {
                                picker.enter(entry);
                            }
                        }
                    });
                });
            if let Some(path) = chosen {
                self.set_project(path);
            } else if open && !cancelled {
                self.picker = Some(picker);
            }
        }

        // 6. Fenêtre d'assembleur.
        if let Some(view) = self.assembly.clone() {
            let mut open = true;
            let mut closed = false;
            egui::Window::new(format!("Assembleur — {}", view.symbol))
                .collapsible(false)
                .resizable(true)
                .default_width(760.0)
                .default_height(560.0)
                .default_pos([420.0, 90.0])
                .open(&mut open)
                .show(ctx, |ui| {
                    if view.missing {
                        ui.colored_label(
                            egui::Color32::from_rgb(230, 200, 80),
                            "fonction introuvable dans l'assembleur généré",
                        );
                    }
                    if let Some(file) = &view.file {
                        ui.label(format!("fichier : {file}"));
                    }
                    if view.unoptimized {
                        ui.colored_label(
                            egui::Color32::from_rgb(230, 200, 80),
                            "version non optimisée : la compilation optimisée inline cette fonction",
                        );
                    }
                    ui.horizontal(|ui| {
                        if ui
                            .button("Copier")
                            .on_hover_text("Copie l'assembleur dans le presse-papiers")
                            .clicked()
                        {
                            ui.ctx().copy_text(view.text.clone());
                        }
                        if ui.button("Fermer").clicked() {
                            closed = true;
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::both().show(ui, |ui| {
                        ui.monospace(&view.text);
                    });
                });
            if !open || closed {
                self.assembly = None;
            }
        }

        // 7. Panneau central : détail du candidat.
        egui::CentralPanel::default().show(ctx, |ui| {
            // Notification de backup (s'affiche après un clic sur "Assembly").
            if let Some(notice) = self.backup_notice.clone() {
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::from_rgb(110, 210, 130), &notice);
                    if ui.button("×").clicked() {
                        self.backup_notice = None;
                    }
                });
                ui.separator();
            }

            let Some(idx) = self.selected else {
                ui.heading("0ass");
                ui.label(
                    "Sélectionne un candidat à gauche pour le lire, l'approuver ou le rejeter.",
                );
                ui.add_space(8.0);
                ui.label("Règles du projet :");
                ui.label("  · la signature de la fonction ne change pas ;");
                ui.label("  · un repli portable subsiste (le programme tourne quand même) ;");
                ui.label("  · les vérifications du projet passent avant et après le patch.");
                ui.add_space(8.0);
                ui.label("0ass ne pousse jamais : le git push reste à ta main.");
                return;
            };
            let c = self.manifest.candidates[idx].clone();
            ui.heading(format!("{} — {}", c.id, c.fn_name));
            ui.label(format!(
                "{} · lignes {}–{} · {} · stratégie {}",
                c.file, c.start_line, c.end_line, c.lang, c.strategy
            ));

            ui.separator();
            egui::Grid::new("detail")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Signaux");
                    ui.label(c.detected.join(" · "));
                    ui.end_row();

                    ui.label("Stratégie");
                    ui.label(&c.strategy_reason);
                    ui.end_row();

                    ui.label("Cibles");
                    ui.label(c.targets.join(", "));
                    ui.end_row();

                    ui.label("Risque");
                    ui.label(format!(
                        "UB {} · portabilité {} · review humaine requise",
                        c.risk.ub, c.risk.portability
                    ));
                    ui.end_row();

                    ui.label("Mesure");
                    match &c.measurement {
                        Some(m) => {
                            ui.label(format!(
                            "{:.2} ms → {:.2} ms ({:+.1} %) · n={} · bruit {:.1} % · MDE {:.1} %",
                            m.before_ms, m.after_ms, m.local_gain_pct, m.n, m.mad_pct, m.mde_pct
                        ));
                        }
                        None => {
                            ui.label("pas encore mesuré");
                        }
                    }
                    ui.end_row();

                    ui.label("Compatibilité");
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            if c.compatibility.signature_preserved {
                                egui::Color32::from_rgb(110, 210, 130)
                            } else {
                                egui::Color32::from_rgb(230, 110, 110)
                            },
                            format!(
                                "signature {}",
                                if c.compatibility.signature_preserved {
                                    "inchangée"
                                } else {
                                    "modifiée"
                                }
                            ),
                        );
                        ui.colored_label(
                            if c.compatibility.fallback_present {
                                egui::Color32::from_rgb(110, 210, 130)
                            } else {
                                egui::Color32::from_rgb(230, 110, 110)
                            },
                            format!(
                                "repli {}",
                                if c.compatibility.fallback_present {
                                    "présent"
                                } else {
                                    "absent"
                                }
                            ),
                        );
                        ui.colored_label(
                            if c.compatibility.compatible {
                                egui::Color32::from_rgb(110, 210, 130)
                            } else {
                                egui::Color32::from_rgb(230, 200, 80)
                            },
                            if c.compatibility.compatible {
                                "vérifications vertes"
                            } else {
                                "vérifications non vertes"
                            },
                        );
                    });
                    ui.end_row();
                });

            if let Some(reason) = &c.compatibility.reason {
                ui.colored_label(egui::Color32::from_rgb(230, 200, 80), reason);
            }
            for r in &c.compatibility.after {
                if !r.ok {
                    ui.colored_label(
                        egui::Color32::from_rgb(230, 110, 110),
                        format!("échec : {} (code {})", r.command, r.exit_code),
                    );
                }
            }

            ui.separator();
            ui.horizontal(|ui| {
                let busy = self.busy.is_some();
                let patch = PathBuf::from(self.patch_path.trim());
                let can_approve = c.status == Status::Review
                    && !busy
                    && !self.patch_path.trim().is_empty()
                    && patch.exists();
                if ui
                    .add_enabled(can_approve, egui::Button::new("Mesurer (A/B, sans commit)"))
                    .on_hover_text(
                        "Applique le patch dans le worktree isolé, compile les deux
                         variantes et lance la comparaison A/B entrelacée.
                         Rien n'est commité.",
                    )
                    .clicked()
                {
                    self.spawn(
                        Task::Measure {
                            id: c.id.clone(),
                            patch: patch.clone(),
                        },
                        "mesure",
                    );
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Assembly"))
                    .on_hover_text(
                        "Demande au compilateur l'assembleur de cette fonction
                         (cargo rustc -- --emit asm) et l'affiche :
                         c'est la référence pour juger une réécriture en asm!.",
                    )
                    .clicked()
                {
                    if let Some(dir) = &self.report_dir {
                        self.backup_notice = Some(format!(
                            "Backup des fichiers avant modification : {}",
                            dir.join("backups").display()
                        ));
                    }
                    self.spawn(
                        Task::Assembly {
                            fn_name: c.fn_name.clone(),
                        },
                        "assembleur",
                    );
                }
                if ui
                    .add_enabled(
                        can_approve,
                        egui::Button::new("Approuver (patch, vérification, commit)"),
                    )
                    .on_hover_text(
                        "Applique le patch dans un worktree isolé, vérifie la compatibilité,\n\
                         mesure le gain, puis crée un commit (un commit par patch).",
                    )
                    .clicked()
                {
                    self.spawn(
                        Task::Approve {
                            id: c.id.clone(),
                            patch,
                        },
                        "approbation",
                    );
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Rejeter"))
                    .clicked()
                {
                    if let Some(dir) = self.report_dir.clone() {
                        match review::reject(&dir, &c.id, "rejeté depuis l'interface") {
                            Ok(()) => {
                                self.status = format!("{} rejeté", c.id);
                                self.reload();
                            }
                            Err(e) => self.status = format!("échec : {e}"),
                        }
                    }
                }
                if ui.button("Ouvrir le patch").clicked() && !self.patch_path.trim().is_empty() {
                    open_path(&PathBuf::from(self.patch_path.trim()));
                }
                if ui.button("Ouvrir la source").clicked() {
                    if let Some(dir) = &self.report_dir {
                        let path = dir.join(&c.file);
                        open_path(&path);
                    }
                }
            });

            ui.add_space(6.0);
            ui.label("Patch :");
            ui.text_edit_singleline(&mut self.patch_path);
        });
    }
}

/// Ouvre un fichier ou un dossier avec l'application par défaut.
fn open_path(path: &Path) {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program).arg(path).spawn();
}

fn main() -> eframe::Result<()> {
    let mut project = std::env::args()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // `--project <chemin>`
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--project") {
        if let Some(value) = args.get(pos + 1) {
            project = PathBuf::from(value);
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("0ass — revue des candidats"),
        ..Default::default()
    };
    eframe::run_native(
        "0ass",
        options,
        Box::new(move |_cc| Ok(Box::new(App::new(project.clone())))),
    )
}
