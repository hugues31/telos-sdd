# Audit fix — reconcile/config invariants

Date : 2026-08-20  
Périmètre : `telos-core` reconcile/config/globs et commandes CLI change/config/check/status/rebuild/test, avec tests spécialisés et goldens directement affectés.

## Causes racines

1. `build_model` accepte volontairement un modèle spec-only avec des scénarios actifs non encore prouvés afin que `rebuild plan/status` reste utilisable à 0/N. Le scellement réutilisait ce modèle sans garde structurelle plus forte : un `reconcile --full` pouvait donc écrire un lock sans preuve, et sans runner.
2. `EditConfig` ne produisait aucun nœud impacté. Une modification globale du runner, des globs, de la politique TDD ou de l’environnement des constraints pouvait ainsi contourner les checks ciblés et les preuves distinctes.
3. La validation des globs et de `agents.hosts` n’existait qu’au staging CLI, avec une compilation différente de celle du walker runtime. Un CHG édité à la main pouvait passer approve/reconcile.
4. `rebuild status` et `telos test` exécutaient la configuration persistée, même lorsqu’un changement approuvé possédait la configuration effective à employer.
5. L’égalité des OID du lock suffisait à afficher `coherent`; les locks produits par une ancienne version mais structurellement incomplets restaient acceptés par `status`, `check --sealed` et export.

## Corrections

### Garde structurelle de scellement

- `require_sealable_structure` est une garde partagée, distincte de `build_model`.
- Chaque scénario d’un intent actif doit posséder au moins un `proves`; le premier scénario manquant est choisi par ID pour une erreur stable et corrective.
- Dès qu’il existe une obligation active, `[test] cmd` doit être non vide.
- La garde s’exécute avant checks/tests/écritures dans reconcile ordinaire et full. Un modèle sans intent actif peut encore être scellé sans runner ni tests.
- Le full valide aussi la configuration persistée.
- `status` refuse de présenter `coherent` pour un ancien lock incomplet. `check --sealed` et export réutilisent la même intégrité. La vue live conserve sa capacité d’observation des états drifted/changing et la reconstruction spec-only reste permissive.

### Configuration effective et portée globale

- `EditConfig` impacte tous les intents et scénarios : toutes les constraints applicables et toutes les cibles de preuve distinctes sont rejouées.
- Un helper CLI construit, dans l’ordre déterministe des changements, la configuration des changements frais approuvés/implementing; les propriétaires multiples forgés sont refusés au lieu d’appliquer un « dernier gagnant » implicite.
- `rebuild status` et `telos test one/all` utilisent cette configuration effective pour la découverte et le runner; les écritures du journal restent dans le workspace réel.

### Validation centralisée

- `Config::validate_self` compile les globs code/tests avec exactement `literal_separator(true)`, sémantique du walker runtime.
- `Config::validate_transition(base, effective)` valide l’effective et interdit les changements réels de `agents.hosts` après normalisation (ordre/doublons tolérés).
- Staging, approve, reconcile autoritatif et consommateurs sealed réutilisent ces validations.
- Un CHG invalide édité à la main reste drafted sans digest lors d’approve; une ré-approbation invalide conserve le digest existant; reconcile refuse sans modifier config, changement ou lock.

## Déroulé TDD

RED séparés observés avant implémentation :

- full et ordinary reconcile acceptaient un scénario actif sans preuve;
- full avec toutes les preuves mais sans runner ne refusait pas;
- approve acceptait un glob forgé invalide et reconcile un changement de hosts forgé;
- EditConfig ne déclenchait ni constraint scoped ni toutes les preuves;
- rebuild status et `telos test` ignoraient le runner staged approuvé;
- status/check/export acceptaient un lock legacy structurellement incomplet.

Chaque RED a ensuite été rendu GREEN par la plus petite garde partagée correspondante. Des cas frontières couvrent aussi : zéro intent actif sans runner, absence totale d’écriture au refus, preuves distinctes, ré-approbation sans mutation et maintien du rebuild spec-only 0/2.

## Fixtures et goldens adaptés

La fixture scellée commune est désormais réellement scellable : deux scénarios prouvés et runner de fixture documenté `git --version`. La fixture `unsealed_fixture` reste inchangée et partielle pour la reconstruction spec-only.

Adaptations directement induites : couverture status/change-flow à 2/2, graphe impact incluant la preuve canonique de SCN-0091, cas remove/context isolés avec INT-0017 draft, diagnostic `telos test` sans runner exercé via un lock legacy produit au bas niveau, et intent éphémère draft pour le test d’ownership/remove.

## Round post-review 1

- Cause : la garde de structure utilisait `cmd.trim().is_empty()`, tandis que `run_tests` et `run_full_tests` utilisaient `cmd.is_empty()`. Une commande uniquement composée d’espaces franchissait donc l’exécution shell et incrémentait mensongèrement `tests_run`.
- Deux RED distincts, full et ordinary/EditConfig sur intents uniquement drafts, ont chacun observé `tests_run: 1` au lieu de `0`.
- Le correctif minimal applique la même sémantique `trim().is_empty()` aux deux exécuteurs; les deux régressions sont GREEN et n’invoquent plus le shell.
- Le test global EditConfig utilise désormais un runner qui journalise les filtres réellement substitués. Trois bindings, dont une cible partagée par deux scénarios, produisent exactement deux lignes distinctes et `tests_run: 2`.
- Aucun test « propriétaires config multiples » n’a été ajouté : l’API CLI naturelle interdit déjà le second claim. Construire ce cas exige de forger manuellement deux CHG et testerait principalement le contournement du format, alors que la branche défensive reste déterministe dans `approved_config_workspace`.

## Round post-review 2

- Finding : `check_witnesses` classait encore le runner avec `is_empty()` alors que la garde structurelle et les deux exécuteurs utilisaient `trim().is_empty()`.
- RED public : un changement ajoute SCN-0108 actif, journalise seulement un run vert (preuve présente, witness rouge dû), puis sa config approuvée est révisée vers un runner composé d’espaces. Reconcile renvoyait à tort `TELOS_SCENARIO_RED_EXPECTED` avant la gate runner.
- Correction minimale : `check_witnesses` emploie désormais `ws.config.test.cmd.trim().is_empty()`. Le même test reçoit exactement `TELOS_TEST_NOT_FOUND`, sans écriture de config, lock ou suppression du CHG.
- La suite reconcile couvre les trois classifications : runner vide et whitespace comme absents, runner non vide comme présent; elle conserve aussi les branches draft-only whitespace à `tests_run: 0` et active/non vide redevable d’un witness.

## Vérification fraîche

- Round post-review 2 : `rtk cargo test -p telos --test reconcile` : 49/49; `rtk cargo test -p telos-core` : 648/648; clippy ciblé core/reconcile avec `-D warnings` et rustfmt ciblé : propres.
- Round post-review 1 : `rtk cargo test -p telos --test reconcile` : 48/48; clippy ciblé `telos-core --all-targets` et `telos --test reconcile` avec `-D warnings` : aucune alerte.
- `rtk cargo test -p telos-core` : 648 tests, 13 suites, tous verts.
- Suites spécialisées reconcile/config/rebuild/status_check/view_export : 95 tests, 5 suites, tous verts.
- Ensemble des autres suites CLI propres hors Task8/Task9 : 313 tests, tous verts.
- `view_server` hors sandbox (ports loopback) : 5/5 verts.
- `rtk cargo clippy -p telos-core --all-targets -- -D warnings` : aucune alerte.
- `rtk cargo clippy -p telos --bins --tests -- -D warnings` : aucune alerte.
- `rustfmt --check` ciblé et `git diff --check` : propres.

## Risques et limites connus

- `rebuild_demo.rs` n’est pas modifié : son bootstrap public scelle encore des intents actifs à 0/2. Son propriétaire Task8 doit livrer l’adaptation cohérente du demo/README/constraint; son échec est attendu jusque-là.
- `acceptance_loops.rs` et `contracts.rs` contiennent les changements Task9 non committés, préservés et exclus. `acceptance_loops` conserve notamment un attendu historique `tests_run: 0` incompatible avec le runner de la fixture scellée.
- Les quatre fichiers Task9, ainsi que README/docs/demo et les changements concurrents export/agents/safe_fs, ne font pas partie du commit focal.
