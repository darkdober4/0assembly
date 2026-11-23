# 0ass

**0ass** repère, dans n'importe quel programme, les fonctions qui peuvent
gagner en vitesse — réécriture portable, SIMD, ou même assembleur — puis
**mesure** chaque proposition avant de la pousser.

Le principe est simple : *une optimisation non mesurée n'est pas une
optimisation*. 0ass propose, l'humain décide, la machine mesure.

---

## Ce que 0ass ne fait pas

| | |
|---|---|
| Il ne modifie jamais votre dépôt sans validation humaine explicite | le travail se fait dans un *worktree* isolé, hors du dépôt |
| Il ne pousse jamais | le `git push` reste à votre main |
| Il ne fusionne jamais sans fast-forward | pas de commit de merge caché |
| Il ne committe jamais un patch incompatible | voir les [règles de compatibilité](#règles-de-compatibilité) |

---

## Installation

```bash
cargo build --release
# binaires : target/release/0ass et target/release/0ass-gui
```

Prérequis : Rust stable (1.82+). L'interface graphique utilise `eframe`
(`egui`) et fonctionne sous Linux, macOS et Windows.

---

## Démarrage rapide

```bash
cd /chemin/vers/mon/projet
0ass init                 # écrit un 0ass.toml à compléter
0ass scan                 # scan statique multi-langages -> rapport
0ass gui                  # interface de revue (tableau + détail + actions)
```

`0ass scan` ne touche à rien : il produit un dossier de rapport **hors du
dépôt** :

```
%LOCALAPPDATA%/0ass/reports/<projet>/<date>-runN/
├── manifest.json     # format d'échange (agent, GUI, outils)
├── summary.md        # le rapport lisible par un humain
├── patches/          # les patchs proposés
├── logs/
└── worktree/         # le worktree isolé (hors du dépôt analysé)
```

---

## Le profil projet : `0ass.toml`

C'est ce qui permet à 0ass de travailler avec **n'importe quel programme** :
il ne connaît pas tous les systèmes de build, il sait lire un profil.

```toml
[project]
name = "demo"
kind = "rust"                 # rust | generic | c | go | python | node
source_roots = ["src"]

[build]
command = "cargo build --release"
binary = "target/release/demo"

[run]
command = "target/release/demo"   # la plus petite charge qui exerce le code visé
warmup = 3
runs = 30

[verify]
check = ["cargo check"]       # l'interface reste compatible ?
test  = ["cargo test"]        # le comportement reste le même ?

# [bench]
# command = "cargo bench"

# [profile]
# linux = "perf record -g -- cargo bench"
# macos = "xcrun xctrace record --template 'Time Profile' --launch -- cargo bench"
```

---

## Règles de compatibilité

**Le code réécrit doit rester compatible avec le reste du programme, et le
programme doit continuer de tourner.**

Un patch n'est jamais commité s'il ne respecte pas ces trois règles :

1. **Interface** — la signature de la fonction est inchangée : le reste du
   programme compile sans être touché.
2. **Repli** — un chemin portable subsiste : le binaire tourne même si le
   chemin optimisé n'est pas pris (CPU sans AVX, autre architecture, build
   portable). Une stratégie par architecture exige une détection à l'exécution
   (`is_x86_feature_detected!`, `#[cfg(target_arch = …)]`) **et** un repli.
3. **Comportement** — les commandes de `[verify]` passent **avant** et
   **après** le patch. Un test qui casse, c'est un refus, pas une discussion.

Ces trois points sont vérifiés mécaniquement, consignés dans le manifeste, et
affichés dans l'interface.

---

## Les langues

Le scan est **générique** : il s'appuie sur un registre de descripteurs
(35+ langages) et ne contient aucune logique propre à un langage.

| Famille | Langages |
|---|---|
| Systèmes | Rust, C, C++, C#, Go, Zig, D, Nim, Objective-C |
| JVM | Java, Kotlin, Scala, Groovy |
| Mobile | Swift, Dart |
| Web | JavaScript, TypeScript, PHP, Vue/Svelte (via JS/TS) |
| Scripts | Python, Ruby, Lua, Perl, R, Julia, MATLAB, Fortran |
| Fonctionnels | Haskell, OCaml, Erlang, Elixir |
| Coquilles | Bash, PowerShell, Cmd, SQL |

Ajouter une langue = ajouter une entrée dans le registre (`lang.rs`).

---

## La mesure

Trois niveaux, du plus fiable au plus informatif :

1. **micro-bench** (`criterion`, banc du projet) — la référence ;
2. **black-box** (temps du processus, entrelacé A-B-A-B) — le compromis par
   défaut ;
3. **profileur** (ETW, `perf`, `xctrace`) — un bonus, désactivé par défaut.

Le protocole A/B entrelace les exécutions (A-B-A-B) pour annuler la dérive de
la machine : 3 exécutions d'échauffement, 30 runs minimum, médiane et MAD.
Un bruit supérieur à 5 % rend la mesure **non fiable** et le résultat est
rejeté. L'effet minimum détectable (MDE) est indiqué à côté du gain.

---

## Les stratégies

| Stratégie | Cible | Risque UB | Gain typique |
|---|---|---|---|
| `bounds_elimination` | portable | low | 5 % – 35 % |
| `branchless` | portable | low | 2 % – 20 % |
| `alloc_hoist` | portable | low | 5 % – 40 % |
| `simd_portable` (`std::simd`) | portable | low | 20 % – 300 % |
| `intrinsics_simd` (`core::arch`) | par archi | medium | 30 % – 400 % |
| `asm_inline` (`asm!`) | par archi | high | variable |
| `measure_first` | portable | low | variable |

Le choix est conservateur : on part de la stratégie la plus sûre qui répond au
signal principal, et on n'atteint jamais l'assembleur sans mesure.

---

## Commandes

```
0ass init                       écrit un 0ass.toml
0ass scan                       scan statique (+ banc si mesurable)
0ass report [--json]            relit le dernier rapport
0ass bench --id A1 --patch p    mesure un patch sans approuver
0ass approve --id A1 --patch p  vérifie, mesure, commite (worktree isolé)
0ass merge                      fusion fast-forward dans la branche courante
0ass gui                        interface de revue
```

---

## L'interface graphique

`0ass gui` ouvre une fenêtre à trois zones :

- **en haut** : le projet, les campagnes, le bouton *Merge fast-forward dans
  main* ;
- **à gauche** : la table des candidats (filtrable), avec le statut et le gain ;
- **au centre** : le détail d'un candidat — signaux, stratégie, risque, mesure,
  et l'état de la compatibilité (signature / repli / vérifications).

Les actions longues (scan, approbation, fusion) tournent en arrière-plan :
l'interface ne se fige pas.

---

## Architecture

```
crates/
├── zeroass_core/     # langages, scan, stratégies, profil, manifeste, résumé
├── zeroass_engine/   # exécution, mesure, campagnes, git, vérification
├── zeroass_cli/      # la commande `0ass`
└── zeroass_gui/      # l'interface `0ass-gui` (egui/eframe)
```

Le **manifeste JSON** est le format d'échange : le moteur l'écrit, l'interface
le lit, un agent peut le consulter. Il ne contient jamais de secret et vit
hors du dépôt analysé.

---

## Licence

MIT. Le code est écrit pour être lu : commentaires explicites, pas de magie.
