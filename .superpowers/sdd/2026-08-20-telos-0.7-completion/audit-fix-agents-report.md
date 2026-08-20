# Pré-audit — correction de la publication des agents

## Cause racine

`PlannedWrite` ne conservait que le chemin et les octets calculés au
préflight. Après le seal, `render` rouvrait chaque cible avec
`create(true).truncate(true)`. Une cible absente au préflight pouvait donc
acquérir un propriétaire avant le rendu et être tronquée; une configuration
utilisateur lue puis mergée pouvait aussi changer entre le préflight et le
rendu et être remplacée par un merge périmé. Enfin, une écriture échouée
pouvait rendre visibles des octets partiels au nom final.

## Correction

- `PlannedWrite` est maintenant une opération typée `CreateOnly` ou
  `MergeExisting`. La seconde conserve les octets exacts lus au préflight.
- Tous les contenus agents sont écrits et `sync_all` dans des fichiers
  siblings réservés par `create_new`; l'écriture utilise directement le
  handle qui a effectué la réservation.
- Chaque traversal reste ancré dans `SafeRoot`, ouvre les ancêtres sans suivre
  les symlinks et conserve le handle du parent jusqu'à la publication.
- Toutes les cibles et tous les chemins parents sont revalidés avant la
  première publication. Les merges sont aussi recomparés juste avant leur
  remplacement.
- Une création est publiée par hard-link atomique: un propriétaire tardif fait
  échouer l'opération sans être remplacé. Un merge validé est publié par rename
  atomique du sibling complet. Le chemin final n'est plus jamais ouvert avec
  `truncate`.
- Une erreur pendant le staging détruit tous les siblings déjà préparés et ne
  publie aucun fichier agent. Les anciens octets d'un merge restent intacts.

## Preuves TDD

RED observé avant le code de production:

```text
error[E0432]: unresolved import `super::render_with_hooks`
```

Les nouveaux tests déterministes couvrent:

- les propriétaires fichiers réguliers apparus tardivement pour les six
  familles utiles Claude/Codex (skill, JSON, `AGENTS.md`, rules);
- les quatre cibles utilisateur mergées modifiées après préflight;
- un échec après écriture partielle du staging d'un merge, avec ancien contenu
  préservé, aucune cible nouvelle publiée et aucun sibling de staging restant;
- le test existant de remplacement tardif d'un parent par un symlink.

GREEN ciblé final:

```text
rtk cargo test -p telos --bin telos commands::agents::tests --no-fail-fast
# 4 passed
rtk cargo test -p telos --test agent_init --no-fail-fast
# 39 passed
rtk cargo test -p telos --test init_ci --no-fail-fast
# 16 passed
rtk cargo test -p telos --bin telos --no-fail-fast
# 36 passed
rtk cargo clippy -p telos --bin telos --tests -- -D warnings
# no issues
```

Les quatre fichiers Rust modifiés passent `rustfmt --check` et
`git diff --check`. Le `cargo fmt --check` global reste perturbé par les
changements Task9 non formatés, notamment dans `telos-core/src/reconcile.rs`
et les tests partagés; ils ont été préservés sans modification et ne font pas
partie de ce correctif.

## Ordre d'init et limite connue

`init` effectue toujours les préflights agents et CI avant la première écriture
du projet, scelle Telos, puis publie les agents et enfin le workflow CI. Garder
la publication post-seal évite de déposer des intégrations hôte si la création
ou le seal de Telos échoue. Étendre `init` avec un protocole de reprise d'une
initialisation déjà scellée changerait le contrat `TELOS_ALREADY_INITIALIZED`
et la gestion des arguments de reprise; ce changement n'est pas introduit dans
ce correctif focal.

Il n'existe pas de transaction CAS portable couvrant plusieurs répertoires et
plusieurs fichiers. Sous le modèle de menace non adversarial déjà documenté
pour le seal, le correctif ferme les interleavings déterministes: validation
globale avant publication, comparaison adjacente à chaque rename et
publication no-replace des créations. Il subsiste un intervalle syscall entre
la dernière lecture d'un merge existant et son rename; un acteur adversarial
qui modifie précisément cette cible dans cet intervalle n'est pas couvert.
