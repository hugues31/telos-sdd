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

Les fichiers Rust du correctif passent `rustfmt --check` et
`git diff --check`. Les changements voisins ont été préservés sans modification
et ne font pas partie du correctif.

## Ordre d'init et reprise post-seal

`init` effectue toujours les préflights agents et CI avant la première écriture
du projet. Il publie ensuite `.telos-init.json`, qui contient un format versionné
et les options agents/CI normalisées, puis crée et scelle Telos, publie les
agents et le workflow CI, et retire enfin le marqueur en vérifiant ses octets et
son identité. Un retry ne reconnaît que le marqueur byte-exact de ses propres
options. Il replanifie depuis les octets courants: les artefacts exacts sont des
no-ops revalidés, les configurations utilisateur sont mergées de façon
idempotente, et un propriétaire non-Telos à un chemin de skill reste intact.
Un workflow CI exact n'est un no-op que dans cette reprise. Sans marqueur, un
projet terminé conserve le contrat `TELOS_ALREADY_INITIALIZED`.

Il n'existe pas de transaction CAS portable couvrant plusieurs répertoires et
plusieurs fichiers. Sous le modèle de menace non adversarial déjà documenté
pour le seal, le correctif ferme les interleavings déterministes: validation
globale avant publication, comparaison adjacente à chaque rename et
publication no-replace des créations. Il subsiste un intervalle syscall entre
la dernière lecture d'un merge existant et son rename; un acteur adversarial
qui modifie précisément cette cible dans cet intervalle n'est pas couvert.

## Review fix round 1

- `StagedWrite` compare maintenant l'identité cross-platform `(dev, ino)` du
  parent capability tenu à celle du parent rouvert. Le remplacement tardif
  d'un ancêtre par une vraie arborescence est refusé, comme les symlinks
  l'étaient déjà; aucun final n'apparaît dans l'ancien arbre déplacé ni dans le
  nouveau.
- Le chemin CI n'écrit plus jamais dans le final réservé. Il utilise le même
  staging sibling complet, `sync_all`, contrôle d'identité et hard-link
  no-replace que les créations agents. Un writer injecté qui écrit 17 octets
  puis échoue laisse le workflow absent.
- Le cleanup compare l'identité du staging avant unlink. Le test déterministe
  qui déplace le staging puis place un propriétaire tardif sous son ancien nom
  prouve que ce propriétaire n'est pas supprimé.
- Le test de reprise injecte l'échec au troisième publish agent après seal,
  vérifie les deux skills déjà publiés, refuse des options différentes, puis
  réussit avec les options exactes sans dupliquer le merge propriétaire. Le
  marqueur disparaît seulement après agents et CI terminés.

RED observés pendant ce round: publication réussie dans un ancêtre déplacé,
workflow final CI partiel, suppression du propriétaire tardif du staging,
six publications répétées au lieu de no-ops, acceptation d'un skill non-Telos,
acceptation d'un no-op agent/CI modifié après préflight, et absence des APIs de
reprise/marqueur. Chaque cas a ensuite été observé GREEN isolément.

La limite adversariale restante est le petit intervalle entre comparaison
d'identité et syscall de link/rename/unlink. Un inode supprimé puis réutilisé
avec le même identifiant dans cet intervalle, ou un marqueur volontairement
forgé byte pour byte, sort du modèle non-adversarial du seal. Sur ReFS,
`cap-fs-ext` expose actuellement un identifiant de fichier 64 bits bien que le
système puisse en fournir 128; cette limite amont est également conservée.

Vérification finale du round:

```text
rtk cargo test -p telos --bin telos --no-fail-fast
# 49 passed
rtk cargo test -p telos --test agent_init --no-fail-fast
# 39 passed
rtk cargo test -p telos --test init_ci --no-fail-fast
# 16 passed
rtk cargo test -p telos --test cli_m1 --no-fail-fast
# 14 passed
rtk cargo clippy --workspace --all-targets -- -D warnings
# no issues
```

Le `cargo fmt --check` global signale seulement le fichier voisin non indexé
`crates/telos/tests/contracts.rs`; le `rustfmt --check` explicite de tous les
fichiers Rust de ce round est vert.

## Review fix round 2

Le marqueur de reprise est passé au format `telos-init-v2`. Il ne contient
plus seulement les options normalisées: il conserve, pour chaque cible cœur
écrite par `init` (`telos.toml`, `bindings.tel`, `counters.toml`,
`.gitattributes` et le lock), les octets initiaux et/ou les octets exacts de
la transaction. Sa phase est explicite: `preparing`, puis `sealed` avec les
octets canoniques du lock.

Une reprise `preparing` valide globalement les shapes des répertoires et
chaque cible comme état initial ou état désiré avant sa première écriture.
Les cibles déjà exactes sont des no-ops; les absentes sont publiées
no-replace et les anciennes ne sont remplacées qu'après comparaison avec le
snapshot. Après création du lock, le marqueur passe atomiquement à `sealed`
par remplacement CAS. L'intervalle déterministe où le lock est déjà publié
mais le marqueur encore `preparing` est lui aussi reprenable.

Une reprise `sealed` exige tous les octets cœur exacts, tous les répertoires
réels et présents, puis recalcule le seal depuis le workspace vivant. Cette
validation a lieu avant les publications agents/CI, à leur frontière, puis
avant la suppression du marqueur. Des octets étrangers, un fichier remplacé
par un répertoire ou un symlink final sont donc refusés avec
`TELOS_CHANGE_STATE_INVALID`, sans écriture et sans consommer le marqueur.

RED déterministes observés avant correction: les retries acceptaient les
octets étrangers de chacune des cinq cibles cœur, un `bindings.tel` devenu
répertoire et un `telos.toml` symlinké vers un owner extérieur. Ils
réécrivaient/re-scellaient ensuite le projet. Les tests verts couvrent aussi
une publication cœur partielle pré-seal, ainsi qu'un échec juste après la
publication du lock mais avant le changement de phase.

Vérification du round:

```text
rtk cargo test -p telos --bin telos
# 54 passed
rtk cargo test -p telos --test agent_init
# 39 passed
rtk cargo test -p telos --test init_ci
# 16 passed
rtk cargo test -p telos --test cli_m1
# 14 passed
rtk cargo clippy --workspace --all-targets -- -D warnings
# no issues
```

Le `cargo fmt --all -- --check` global reste perturbé uniquement par le
fichier voisin non indexé `crates/telos/tests/contracts.rs`; le check
`rustfmt` ciblé et `git diff --check` sont verts. Les limites adversariales
restent celles du round précédent: pas de CAS global portable sur plusieurs
fichiers, petite fenêtre lecture/rename pour un remplacement existant,
identité d'inode réutilisable, et marqueur pouvant être forgé byte pour byte
par un acteur qui contrôle déjà le dépôt. Deux contenus byte-identiques sont
nécessairement indistinguables; ces cas sortent du modèle non-adversarial du
seal.

## Review fix round 3

Le marker n'est plus seulement relu à la fin d'une phase. Deux transitions
CAS persistées précèdent désormais les publications qu'elles autorisent:
`preparing -> core_writing` avant le premier répertoire/fichier cœur, puis
`sealed -> integrating` avant le premier artefact agent/CI. Les deux phases
intermédiaires sont reprises directement après un crash; une invocation qui
n'a pas elle-même effectué la transition ne peut pas considérer la phase
désirée déjà présente comme un no-op.

Chaque frontière exige un marker final régulier, ouvert nofollow, portant les
octets exacts — phase incluse — lus au préflight. Le CAS stage les nouveaux
octets dans un sibling complet puis recompare strictement les anciens octets
avant le rename. `core_writing` est encore revalidé immédiatement avant le
premier syscall de création cœur; `integrating` est revalidé avant agents,
entre agents et CI, puis avant le cleanup.

Les RED injectent, au moyen des hooks de frontière, trois mutations du marker
après le préflight: octets étrangers, avance vers l'exacte phase suivante, et
remplacement du fichier par un répertoire. Ils couvrent séparément un marker
`preparing` juste avant la première écriture cœur et un marker `sealed` juste
avant les intégrations. Le snapshot non-Git pris par le mutateur reste
byte-identique après le refus: aucun nouvel artefact cœur/agent/CI n'apparaît,
et le marker ou owner étranger reste en place. Le RED initial était l'absence
de la frontière testable (`no run_with_boundary_hooks`); les deux tests sont
ensuite passés GREEN pour les six interleavings.

Vérification ciblée du round:

```text
rtk cargo test -p telos --bin telos commands::init::tests
# 9 passed
rtk cargo test -p telos --test agent_init
# 39 passed
rtk cargo test -p telos --test init_ci
# 16 passed
rtk cargo test -p telos --test cli_m1
# 14 passed
rtk cargo clippy --workspace --all-targets -- -D warnings
# no issues
```

Le `rustfmt --check` ciblé et `git diff --check` sont verts. La limite
adversariale est réduite à l'intervalle syscall non supprimable entre la
dernière comparaison du marker et le rename/création suivant; une transaction
globale multi-fichier reste non portable. Un acteur capable de forger le
marker et tous les snapshots exacts reste hors du modèle de menace du seal.
