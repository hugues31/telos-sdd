# telos-sdd — Spécification de design

**Date** : 2026-08-19 · **Statut** : approuvé (session de brainstorming) · **Implémentation** : Rust

---

## 1. Thèse

**telos-sdd** est un framework de développement assisté par IA dans lequel la source de vérité d'un projet n'est pas son code mais son **telos** : une base d'intents typée, à intégrité référentielle dure, versionnée dans git. Le code n'est qu'*une* solution possible du telos ; l'étoile polaire (et une fonctionnalité livrée) est de pouvoir supprimer `src/` et reconstruire un projet **conforme** — tous les scenarios verts, toutes les constraints satisfaites — depuis la seule base.

Aucun humain ni agent ne peut modifier le code ou la base hors protocole sans que l'état devienne `DRIFTED`, ce qui bloque le workflow jusqu'à régularisation. Le CLI `telos` joue le rôle de moteur de base de données : il est le seul chemin d'écriture légitime, valide l'intégrité à chaque opération, et scelle les états cohérents par hash.

Positionnement vs l'existant : Spec Kit, Kiro, OpenSpec et BMAD convergent vers des specs Markdown semi-structurées (user stories + EARS) non requêtables ; Tessl pousse le « spec as source » mais reste du Markdown outillé. **Personne n'offre une base de spec typée, requêtable, à intégrité dure et à état scellé — c'est le créneau de telos-sdd.** Le framework en reprend le meilleur : la séparation des rôles et le challenge de bmad, la discipline process de superpowers, le delta-first d'OpenSpec, la notation EARS de Kiro/Spec Kit.

## 2. Vocabulaire

| Terme | Définition |
|---|---|
| **Telos** | La base d'intents d'un projet : l'ensemble des notions, intents, scenarios, constraints, bindings et changes. Source de vérité. |
| **Notion** | Terme du langage omniprésent (DDD) avec attributs typés. Kinds : `actor`, `entity`, `value`, `event`, `state`. |
| **Intent** | Comportement ou propriété exigée du système, énoncé en EARS typé, avec son *telos* (le pourquoi). |
| **Scenario** | Critère d'acceptation d'un intent : Given/When/Then typé. |
| **Constraint** | Borne sur les solutions acceptables (stack, architecture, qualité, sécurité, convention). |
| **Change** | Transaction de mutation du telos : delta staged → approbation liée au digest → implémentation → reconcile. |
| **Binding** | Lien maintenu par le rapprochement : `implements` (fichier → intents), `proves` (test → scenario). |
| **Seal** | L'état scellé, matérialisé par `telos/telos.lock` : blob OIDs git de la spec et des fichiers de code liés. |
| **Drift** | Modification hors protocole de la spec ou du code lié. |
| **Reconcile** | Le rapprochement : application atomique d'un change, vérification totale, re-scellement. |
| **Coherent** | État où le lock est valide : spec, code et bindings conformes aux hashes scellés. |

Le vocabulaire officiel (concepts, CLI, contenu de la base) est en **anglais**.

## 3. Modèle de données

### 3.1 Entités

- **Notion** — clé naturelle PascalCase (`Invoice`). Champs : `kind`, `def` (définition courte), `attr`s typés (`string`, `int`, `decimal`, `money`, `bool`, `enum(...)`, `date`, `datetime`, `ref`), `rel`s nommées vers d'autres notions.
- **Intent** — ID `INT-NNNN`. Champs : `title`, `status` (`draft` | `active` | `deprecated`), `telos` (rationale), `statement` (EARS typé, §4.2), scenarios imbriqués, relations.
- **Scenario** — ID `SCN-NNNN`, imbriqué dans son intent. Steps typés (§4.3).
- **Constraint** — ID `CON-NNNN`. Champs : `kind` (`stack` | `architecture` | `quality` | `security` | `convention`), `rule` (déclaratif ou expression), `scope` (`global` ou liste d'intents), `check` optionnel (commande shell dont l'exit code vérifie la constraint machine).
- **Change** — ID `CHG-NNNN`. Champs : `motivation`, liste ordonnée d'opérations staged (`add`/`edit`/`remove` + payload), `approved_digest`, journal des runs de test (avec témoins rouges scellés, §7.2.4), `status` (`open` → `drafted` → `approved` → `implementing` → `reconciled`, ou `abandoned`).
- **Binding** — paires `implements(path, INT-id)` et `proves(test-ref, SCN-id)`. Écrites uniquement par le CLI.

Les IDs sont attribués par le CLI, zéro-paddés sur 4 chiffres, jamais réutilisés.

### 3.2 Relations du graphe

| Relation | De → vers | Rôle |
|---|---|---|
| `refines` | intent → intent | Hiérarchie |
| `requires` | intent → intent | Dépendance ; donne l'ordre topologique du rebuild |
| `excludes` | intent ↔ intent | Incompatibilité déclarée ; matière première du challenger |
| `constrains` | constraint → intents \| global | Portée des constraints |
| `verifies` | scenario → intent | Un scenario appartient à exactement un intent |
| `uses` | intent/scenario → notion | **Dérivée automatiquement** des statements et steps, jamais déclarée |
| `implements` | fichier → intent | Binding |
| `proves` | test → scenario | Binding |

### 3.3 Règles d'intégrité (comportement « moteur de bdd »)

1. **Aucune référence pendante**, nulle part — rejet à l'écriture, comme une clé étrangère.
2. Supprimer une entité référencée est rejeté (ou cascade explicite `--cascade`, qui staged la suppression des référents dans le même change).
3. **Cycles interdits** sur `requires` et `refines`.
4. Un intent ne passe `active` qu'avec ≥ 1 scenario ; au reconcile, chaque scenario d'un intent actif doit avoir ≥ 1 test `proves` vert.
5. **No code without telos** : au reconcile, tout fichier couvert par les globs `code` doit apparaître dans ≥ 1 `implements` ; tout fichier des globs `tests` dans ≥ 1 `proves`. Le code orphelin bloque le rapprochement — c'est ce qui force le strict minimum pour satisfaire le set d'intents.

## 4. Le langage `.tel`

### 4.1 Principes

- **Un seul langage de surface**, parser Rust maison (chumsky/pest), avec des micro-grammaires typées par domaine.
- Le `.tel` est une **forme canonique émise par le CLI** (ordre des champs, indentation et quoting déterministes). Personne ne l'écrit à la main : les agents mutent via le CLI avec payloads JSON ; les humains le lisent en review de PR. Une édition manuelle = drift.
- Toutes les références (notions, attributs, valeurs d'enum, IDs) sont résolues et vérifiées par le moteur au parse et à l'écriture.

### 4.2 Statements EARS typés (5 gabarits)

| Gabarit | Forme |
|---|---|
| `ubiquitous` | `system shall <action>` |
| `event-driven` | `when <EventNotion> [on <Notion>]` … `system shall <action>` |
| `state-driven` | `while <Notion.state = valeur>` … `system shall <action>` |
| `unwanted` | `if <condition>` … `system shall <action>` |
| `optional` | `where <feature-flag>` … `system shall <action>` |

Les slots `<…>` référencent des notions/attributs déclarés ; `<action>` est une phrase d'action contenant des références résolues (`set Invoice.state = settled`, ou une clause libre courte si l'action n'est pas exprimable formellement — la clause libre est autorisée uniquement dans l'action, jamais dans les triggers/conditions).

### 4.3 Scenarios typés

- `given` — instances de notions avec états d'attributs : `given Invoice { state: open, balance: "120.00 EUR" }` (plusieurs lignes = And).
- `when` — un événement (notion `event`) avec payload typé.
- `then` — assertions dans le mini-langage d'expressions.

### 4.4 Mini-langage d'expressions

Utilisé dans les `then`, les conditions EARS et les `rule` machine-checkables : références `Notion.attr`, littéraux (string, int, decimal, money `"120.00 EUR"`, symboles d'enum, bool, date), opérateurs `== != < <= > >=`, `in`, `and or not`, parenthèses. Pas de fonctions ni de quantificateurs en v1.

### 4.5 Exemple canonique

```
notion Invoice entity {
  def  "A bill issued to a Customer for delivered work."
  attr state   enum(open, settled, cancelled)
  attr balance money
  rel  issued-to -> Customer
}

intent INT-0042 "Invoice payment marks it settled" {
  status active
  telos  "Customers must see immediately that their debt is cleared."
  statement event-driven {
    when   PaymentReceived on Invoice
    system shall set Invoice.state = settled
  }
  requires INT-0017

  scenario SCN-0107 "full payment settles the invoice" {
    given Invoice { state: open, balance: "120.00 EUR" }
    when  PaymentReceived { amount: "120.00 EUR" }
    then  Invoice.state == settled
  }
}

constraint CON-0003 architecture "Hexagonal boundaries" {
  rule  "Domain code must not import adapter modules."
  scope global
  check "scripts/check-imports.sh --layer domain"
}
```

## 5. Stockage et layout

La spec est un citoyen de première classe, visible à la racine du repo.

```
telos/
  telos.toml          # config projet (CLI-géré, éditable via `telos config`)
  telos.lock          # le seal — écrit uniquement par le CLI, versionné (analogie Cargo.lock)
  notions/Invoice.tel
  intents/INT-0042.tel      # scenarios imbriqués dans le fichier de leur intent
  constraints/CON-0003.tel
  bindings.tel              # implements + proves, CLI-géré, reviewable en PR
  changes/CHG-0007.tel      # changes ouverts (delta staged + journal) ; supprimés au reconcile
```

- **Moteur en mémoire** : le CLI charge le telos à chaque invocation (parse + validation + index de graphe) ; à l'échelle visée (< 10 000 entités) c'est de l'ordre de la milliseconde en Rust. Le serveur web garde le modèle en mémoire avec un file-watcher. Un cache pourra être ajouté plus tard sans changer le modèle.
- **`telos.toml`** : `[code] globs`, `[tests] globs`, `[test] cmd = "cargo test {filter}"` (gabarit validé d'un direct process argument vector, pas une commande shell), `[policy] tdd = "strict" | "advisory"`, `[agents]`.
- **`telos.lock`** : digest de la spec et **blob OIDs git** de chaque fichier `.tel` et de code lié — une seule identité d'octets dans tout le système, celle de git après filtres `.gitattributes` (pas de faux drift sur une fin de ligne, vérifiable à la main via `git cat-file`) ; plus version de l'outil et ID du change scellant. Les conflits de merge git sur le lock se résolvent par re-scellement prouvé (§7.4). Le snapshot des OIDs spec/code est pris avant checks/tests, revalidé après leur exécution et aux frontières de publication ; le lock reprend les OIDs de code/preuve effectivement exécutés, jamais un re-hash tardif non prouvé.
- **Chemins de dépôt** : toute entrée CLI/`.tel`/journal/lock est un chemin relatif portable et normalisé (`/`, composants normaux uniquement, sans racine/préfixe, `.`, `..`, backslash, colon ou contrôle). Hash, lecture, écriture et restauration répètent la validation avec une capability-anchored, no-follow repository traversal ; un symlink interne ne peut donc pas rediriger Telos hors du dépôt.
- **Publication** : init, merges d'owners agents et export utilisent des siblings CSPRNG, authentifient identité/contenu avant remplacement ou nettoyage, et conservent les owners concurrents ordinaires. L'export authentifie un seul snapshot scellé et la chaîne complète d'identité de ses parents avant promotion. Les blocks agents mal formés sont refusés ; un owner existant est publié par CAS contenu+identité.
- **Modèle de menace** : le seal détecte la négligence (humain, agent, IDE), il ne résiste pas à un adversaire qui forgerait les OIDs. Le confinement par capability/no-follow couvre les chemins malformés et redirections symlink. Les garanties CAS/publication couvrent l'ordinary same-UID concurrency (sauvegarde IDE, collision, rename concurrent non hostile), mais pas un processus malveillant du même UID capable de substituer intentionnellement une entrée entre deux syscalls ; un nom CSPRNG réduit les collisions, ce n'est pas un secret d'autorisation.
- **Collaboration** : un fichier texte par entité → diffs lisibles, reviews de PR normales, conflits de merge rares et localisés ; après résolution manuelle d'un conflit, `telos check` revalide l'intégrité totale.

## 6. États du projet

| État | Définition | Effet |
|---|---|---|
| `COHERENT` | Lock valide : spec, code lié et bindings conformes aux hashes ; intégrité totale verte | Tout est permis |
| `CHANGING` | ≥ 1 change ouvert ; les fichiers touchés sont revendiqués par le change | Workflow normal |
| `DRIFTED` | Spec ou code lié modifié hors de tout change | Les opérations d'avancement (open, approve, reconcile, rebuild, export doc) sont **refusées** |

Sorties du drift : `status` publie un token du lock complet, du scope exact
(chemin + kind) et des OID live correspondants, puis
`telos adopt [--into CHG-…] --expected-state <token>`
(le travail est capturé comme change légitime — on ne perd jamais rien) ou
`telos revert --expected-state <token>` (restauration de l'état scellé via
git). Les agents doivent passer l'exact displayed digest or drift token ; une
valeur manquante ou périmée ferme le guard et une valeur périmée refuse le CLI
à la frontière de mutation. La route humaine sans flag reste compatible mais
se lie à sa première observation et revalide avant d'écrire. Le statut est
recalculé, jamais stocké.

## 7. Workflow

### 7.1 Machine à états d'un change

`open → drafted → approved → implementing → reconciled` (ou `abandoned` à tout moment). Le telos de base n'est jamais muté directement : les opérations sont staged dans le change (overlay) et appliquées atomiquement au reconcile.

### 7.2 Le cycle en 5 phases

1. **Observe** — `telos view` ou `telos status` : l'utilisateur voit le graphe, la couverture, l'état.
2. **Challenge** — l'utilisateur demande une évolution à son agent. Skill `telos-challenger` : `telos change open "<motivation>"`, parcours **dur** du graphe (`query`, `impact` — jamais toute la spec en contexte), classification de la demande : *faisable* / *infaisable* (constraint violée) / *incohérente* (conflit `excludes` ou contradiction avec un intent actif) / *ambiguë* → questions de cadrage à la bmad, une par une. Le delta est staged via `telos add|edit|remove … --change`, le moteur validant chaque opération.
3. **Approve** — `telos change diff` produit le delta lisible et son digest ; l'agent déclenche `telos change approve … --expected-digest <digest affiché>`, qui revalide cette valeur à la frontière d'écriture et lie l'approbation au **digest du delta**. Toute modification ultérieure du delta invalide l'approbation. Jamais d'« équivalence sémantique » jugée par un LLM.
4. **Implement** — skill `telos-implementer`, scenario par scenario, DDD+TDD : le code du domaine nomme les notions ; test rouge d'abord (nommé avec l'ID du scenario), constaté par `telos test SCN-…`, qui hash le fichier avant le process, re-hash après, puis seulement journalise le run **et scelle le témoin rouge** : le blob OID exact du fichier de test exécuté. Les mêmes bytes doivent passer au vert — un runner qui réécrit son test ou une édition concurrente refuse sans journal ; modifier le test entre le rouge et le vert invalide le témoin et exige un nouveau rouge. Puis implémentation minimale jusqu'au vert. En `tdd = "strict"`, le reconcile refuse un scenario sans témoin rouge intact avant le vert. Bindings au fil de l'eau : `telos bind <path> INT-…`.
5. **Reconcile** — `telos change reconcile` : capture le snapshot spec/code avant checks/tests, applique le delta seulement après leur succès et l'égalité des OIDs, revalide toute l'intégrité (§3.3), lance les `check` des constraints, vérifie no-code-without-telos, publie un `telos.lock` lié au snapshot prouvé, revalide à la frontière, puis ferme le change → `COHERENT`. **Un test flaky ne scelle jamais** : `retry-until-green` n'existe pas, même en option — un test `proves` intermittent se répare par un change.

### 7.3 Contexte borné

`telos context INT-0042` compile le pack de travail d'un agent : l'intent, ses scenarios, les constraints applicables, les définitions des notions utilisées, ses bindings, ses voisins à 1 saut, et les constraints globales. **C'est l'unité de contexte des agents — jamais la spec entière.** Les agents itèrent intent par intent.

### 7.4 Cas particuliers

- **Merge git de deux branches scellées** : le lock est conflictuel → `telos change reconcile --full` : revalidation d'intégrité complète + exécution complète des obligations et comportements actifs + re-scellement. S'il existe au moins un intent actif, le runner exécute la suite entière exactement une fois et elle doit être verte ; s'il n'en existe aucun, il n'y a aucune obligation de test et le runner n'est pas invoqué (`tests_run = 0`). Ce n'est pas un bypass : il exige la preuve totale de toutes les obligations actives.
- **Changes concurrents** : autorisés ; un fichier ne peut être revendiqué que par un change à la fois.

## 8. Surface CLI

Toutes les commandes acceptent `--json` — enveloppe stable `{ok, command, result, error: {code, message, hint}, next_actions}`. C'est l'API des agents ; les messages d'erreur sont correctifs (« notion `invoice` inconnue ; la plus proche est `Invoice` »).

| Groupe | Commandes |
|---|---|
| Cycle de vie | `telos init [--agents claude,codex] [--ci github]` · `status` · `check [--sealed]` · `config` · `version` |
| Lecture / graphe | `show <id>` · `list <type>` · `query <type> [--using N] [--status s] [--triggered-by E] …` · `impact <id>` · `context <id>` |
| Mutations (staged) | `add \| edit \| remove <entity> --change <id>` — payload JSON sur stdin |
| Change | `change open \| diff \| approve <id> [--expected-digest SHA256] \| abandon \| list` · `change reconcile [--full]` |
| Tests & bindings | `test <SCN-id \| --all>` · `bind <path> <INT-id>` |
| Drift | `adopt [--into <id>] [--expected-state SHA256]` · `revert [--expected-state SHA256]` |
| Vue & doc | `view [--port N] [--export <dir>]` |
| Rebuild | `rebuild plan` · `rebuild status` |

`telos init --ci github` génère un workflow CI exécutant `telos check --sealed` : le merge sur main exige un état cohérent scellé.

Deux contrats sont **gelés dès M1** — les skills routent dessus, sans interprétation :

- **Error codes** énumérés et stables : `TELOS_DRIFT_DETECTED`, `TELOS_APPROVAL_STALE`, `TELOS_REFERENCE_UNKNOWN`, `TELOS_SCENARIO_RED_EXPECTED`, `TELOS_TEST_SEALED`, `TELOS_ORPHAN_CODE`, `TELOS_CONSTRAINT_FAILED`, `TELOS_CHANGE_STATE_INVALID`, `TELOS_FILE_CLAIMED`, … — chacun avec son `hint` correctif.
- **Schéma de `status --json`** documenté : `state` (`coherent` | `changing` | `drifted`), changes ouverts (id, status, obligations restantes), drift éventuel (`paths`, proposition d'`adopt [--into]`, et ajout 0.7 explicite `token`), compteurs de couverture. Les `next_actions` de drift portent ce même token exact.

## 9. Agents et skills

`telos init` dépose trois skills identiques en contenu pour les deux hôtes (Claude Code : `.claude/skills/telos*/` ; Codex : `AGENTS.md` + fichiers skill). Le CLI est déterministe et **ne fait aucun appel LLM** ; l'intelligence vit dans les agents hôtes.

1. **`telos`** (routeur) — point d'entrée obligatoire : lit `telos status --json` et route vers la bonne phase. Interdit toute édition manuelle de `telos/` et impose le passage par le CLI.
2. **`telos-challenger`** — phase Challenge (§7.2.2). Ne touche jamais au code.
3. **`telos-implementer`** — phase Implement (§7.2.4). Ne modifie jamais le delta approuvé (sinon l'approbation tombe).

La séparation challenger/implementer reprend le meilleur de bmad (rôles qui se challengent) sans simulation d'équipe : deux phases outillées, pas des personnages.

En plus des skills, `telos init` dépose un **guard** : hook préventif (PreToolUse pour Claude Code, équivalent Codex) qui refuse en direct toute édition manuelle de `telos/` par un agent, et présente les décisions humaines (`change approve`, `adopt`, `revert`) comme des prompts de permission natifs. Le skill passe le digest/token exact affiché dans l'argument public ; le guard le recalcule depuis le dépôt et refuse toute forme absente, périmée, composée ou imbriquée. Les fichiers hôtes fusionnés exigent zéro ou un block Telos bien formé et utilisent un CAS atomique contenu+identité pour ne pas écraser une sauvegarde IDE après validation. La détection de drift (§6) reste le filet de sécurité ; le guard rend le drift exceptionnel — humain, IDE ou script, plus jamais un agent qui suit les skills.

## 10. Serveur web et documentation

`telos view` : serveur local vivant (axum, loopback uniquement, lecture seule), modèle en mémoire + file-watcher. Pages, toutes croisées et cliquables :

- **Dashboard** — état du projet, changes ouverts, drift éventuel ;
- **Graphe** interactif — notions/intents/constraints, filtrable par relation ;
- **Page intent** — statement EARS, telos, scenarios, constraints applicables, fichiers `implements`, tests `proves` ;
- **Glossaire** des notions ;
- **Matrice de couverture** — intent × scenario × test : les trous sautent aux yeux.

`telos view --export <dir>` : export statique des mêmes pages (HTML autonome) — publication CI, GitHub Pages, partage sans CLI. Le modèle et l'état sont réauthentifiés en un snapshot scellé unique avant rendu ; l'identité no-follow de toute la chaîne parent de la destination est recapturée avant la promotion atomique. Une sauvegarde modèle ou rotation parent concurrente exporte exactement l'ancien snapshot scellé ou refuse, jamais des bytes nouveaux étiquetés cohérents ni un succès vers une destination devenue invisible.

## 11. `telos rebuild`

**Contrat** : reconstruit un projet **conforme** — tous les scenarios verts, toutes les constraints satisfaites — depuis la seule base Telos. La conformité est comportementale ; l'identité structurelle dépend de la richesse des constraints capturées. Le rebuild est ainsi le **banc de mesure de la qualité de la spec**.

- `rebuild plan` — ordre topologique des intents (via `requires`) + pack de contexte de chacun ;
- l'agent implémenteur exécute le plan intent par intent avec la machinerie normale (red-green, bind, reconcile par lots) ;
- `rebuild status` — progression : scenarios verts / total ; chaque `TestRef` distinct est exécuté une fois globalement, en ordre déterministe, puis le même résultat est projeté vers tous les scenarios qui le partagent.

## 12. Non-goals (v1)

- Pas de runtime hébergé, pas d'appels LLM depuis le CLI.
- Pas de vérification formelle (model checking, SMT) — le mini-langage d'expressions v1 est volontairement pauvre.
- Portée mono-repo ; pas de registry de specs partagées.
- Pas de merge sémantique : les conflits git se résolvent à la main puis se revalident.
- Pas d'édition de la spec via le serveur web (lecture seule).

## 13. Risques identifiés

| Risque | Mitigation |
|---|---|
| Effort de granularité : tout passer par des changes peut sembler lourd pour les petits projets | Le challenger sait produire des deltas minimaux ; `tdd = "advisory"` allège |
| Convention de filtre de tests fragile selon les écosystèmes | `{filter}` configurable + convention ID-dans-le-nom, documentée par skill |
| Coût réel d'un rebuild complet (heures d'agent, tokens) | `rebuild status` reprend où on s'est arrêté ; reconcile par lots |
| DSL inconnu des LLM | Les agents n'écrivent jamais le `.tel` (JSON via CLI) ; grammaire documentée dans les skills |
| Clause libre dans les actions EARS = prose résiduelle | Autorisée uniquement dans `<action>` ; les triggers/conditions/assertions restent formels |

## 14. Roadmap MVP

Les trois **boucles d'acceptation** sont commitées dès M1 comme tests e2e `#[ignore]`, dé-ignorés jalon par jalon — le critère de done de la roadmap est exécutable : **feature** (open → challenge → approve → red/green → reconcile → `COHERENT`), **drift** (édition hors protocole → `DRIFTED` → `adopt` → même boucle), **merge** (deux branches scellées → conflit de lock → `reconcile --full`). Si l'usage quotidien exige de penser aux hashes ou au lock, l'abstraction a échoué et un test doit le montrer.

| Jalon | Contenu |
|---|---|
| **M1 — Moteur** | Parser `.tel`, modèle en mémoire, intégrité, `init/status/check/show/list/query/impact`, lock/seal (blob OIDs), error codes + schéma `status --json` gelés, boucles e2e (`#[ignore]`) |
| **M2 — Transactions** | `change open/diff/approve/reconcile`, mutations staged, `adopt/revert`, détection de drift |
| **M3 — Agents** | Les 3 skills (Claude Code + Codex), guard, `context`, `test`, `bind`, protocole red-green à témoins scellés |
| **M4 — Vue** | `view` + `--export` |
| **M5 — Preuve** | `rebuild`, mode CI (`--ci github`), projet démo reconstruit publiquement |
