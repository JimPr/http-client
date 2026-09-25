# Comment exécuter une requête HTTP dans Zed

## Prérequis

- Zed installé.
- Rust et Cargo installés.
- Un checkout de ce dépôt. Il est nécessaire pour installer le CLI.

## Étapes

### 1. Installer l'extension locale

Dans Zed, ouvrez la palette de commandes et lancez **`zed: install dev extension`**.
Sélectionnez la racine du checkout, celle qui contient `extension.toml`.

### 2. Installer le CLI

Depuis la racine du checkout de l'extension, exécutez :

```sh
cargo install --path cli --locked
```

Le binaire est installé dans `~/.cargo/bin`. Ajoutez ce répertoire à votre `PATH`
si nécessaire, puis redémarrez complètement Zed :

```sh
export PATH="$HOME/.cargo/bin:$PATH"
```

### 3. Ajouter la tâche Zed

Dans Zed, ouvrez la palette de commandes puis lancez **`zed: open tasks`**. Ajoutez
la tâche définie dans [`.zed/tasks.json`](../.zed/tasks.json) à votre fichier de
tâches global `~/.config/zed/tasks.json`.

> **Note** : si vous préférez limiter la tâche à un seul projet, ajoutez la même
> définition dans `<racine-du-projet>/.zed/tasks.json`.

### 4. Exécuter les exemples publics

Ouvrez [`examples/github-api.http`](../examples/github-api.http). Cliquez sur la
flèche à gauche de `GET`, puis choisissez **HTTP: run request at cursor**.

Vous devez voir dans le terminal intégré :

- l'URL `raw.githubusercontent.com` ;
- le statut `200` ;
- le contenu JSON brut de `http-client.environments.example.json`.

Placez ensuite le curseur sur `POST` et relancez la même tâche. Le terminal doit
afficher un statut `200` et le HTML produit par l'API Markdown publique de GitHub.

### 5. Configurer les environnements du projet

Dans le projet contenant vos fichiers `.http`, créez le fichier de configuration :

```sh
cp /chemin/vers/http-client/http-client.environments.example.json \
  http-client.environments.json
```

Adaptez les URL et valeurs partageables. Les environnements sont ordonnés :

```json
{
  "version": 2,
  "environments": [
    {
      "name": "local",
      "variables": {
        "API_BASE_URL": "http://localhost:8080"
      }
    },
    {
      "name": "staging",
      "variables": {
        "API_BASE_URL": "https://api.staging.example.test"
      }
    }
  ]
}
```

À chaque exécution, Zed ouvre le terminal avec la liste des environnements. Entrez
le numéro souhaité ou appuyez sur <kbd>Entrée</kbd> pour choisir le premier.

> **Note** : sans `http-client.environments.json`, aucune liste n'est affichée : la
> requête est lancée directement.

### 6. Conserver les secrets hors Git

Créez `http-client.environments.private.json` à la racine du projet pour les tokens
et autres valeurs locales. Il surcharge les valeurs du fichier public pour
l'environnement choisi et est ignoré par Git :

```json
{
  "version": 2,
  "environments": [
    {
      "name": "staging",
      "variables": {
        "API_TOKEN": "valeur-locale"
      }
    }
  ]
}
```

Utilisez ensuite les variables dans un fichier HTTP :

```http
GET {{API_BASE_URL}}/status
Authorization: Bearer {{API_TOKEN}}
```

## Vérification

Après l'exécution, le terminal intégré affiche l'environnement choisi, l'URL finale,
le statut HTTP, les en-têtes et le corps de la réponse.

## Problèmes courants

- **`zed-http was not found on PATH`** : ajoutez `~/.cargo/bin` au `PATH`, puis
  redémarrez Zed.
- **Aucun sélecteur d'environnement** : créez
  `http-client.environments.json` à la racine du worktree contenant le fichier
  `.http`.
- **Variable non définie** : ajoutez-la dans l'environnement sélectionné, dans le
  fichier privé, ou avant la requête avec `@nom = valeur`.
