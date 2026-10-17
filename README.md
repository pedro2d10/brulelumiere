# BrûleLumière

**BrûleLumière** est un éditeur de projet laser autonome, écrit en Rust, inspiré
de LightBurn. Il réunit le dessin vectoriel, les calques de découpe, la
génération de G-code, le pilotage d'une machine GRBL et la vectorisation
d'image dans une interface unique au style LightBurn.

> Le nom est la traduction littéraire de « LightBurn » (*light* = lumière,
> *burn* = brûler).

## Fonctionnalités

- **Dessin vectoriel** : rectangle, ellipse, polygone régulier, tracé à la plume.
- **Édition de nœuds** : sommets et poignées de Bézier, ligne ↔ courbe, scinder, supprimer.
- **Transformations** : déplacer, échelle, rotation (±15°/±90°), miroir H/V,
  alignements (gauche/droite/haut/bas, centrage), grouper / dégrouper.
- **Historique** : annuler / rétablir complet (Ctrl+Z / Ctrl+Y).
- **Presse-papiers** : couper, copier, coller, dupliquer (Ctrl+X/C/V/D).
- **Calques de découpe** : couleur, vitesse, puissance min/max, passes, ordre
  de découpe (priorité), modes découpe/scan/image.
- **Import / export** : `.lbrn2` (format LightBurn), SVG et DXF.
- **Texte vectoriel**, **tableaux** (grille), **opérations booléennes**
  (union / intersection / différence) et **décalage de contour** (offset).
- **Vectorisation d'image** (« caméra ») : PNG/JPEG → contours de découpe par
  marching squares, seuil réglable, simplification.
- **G-code GRBL** : génération, estimation de durée, aperçu et export `.gcode`.
- **Machine** : connexion série GRBL, envoi de job, cadre, jog, pause/reprise.
- **Navigation** : zoom molette, pan au bouton du milieu, zoom-ajusté / sélection.
- **Stockage** fiable : `project.json` avec écriture atomique, sauvegardes
  tournantes et garde de version (crate `bl-io`).

## Structure

```
crates/
  brulelumiere/   application GUI (eframe/egui)
  bl-io/        magasin de configuration : save atomique, backups, version guard
```

## Compiler et lancer

```bash
cargo run --release          # lance BrûleLumière
cargo test                   # exécute la suite de tests
```

## Licence

MIT — voir [LICENSE](LICENSE).

---

Ce projet n'est pas affilié à LightBurn Software ; il s'en inspire pour
l'ergonomie et la compatibilité de format.
