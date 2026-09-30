# Tests sans matériel (backend factice)

Le backend factice simule un RSP1B : bruit gaussien à niveau d'antenne réglable,
gain par bande, AGC, écrêtage et surcharge, gain total publié pour le niveau RF.
Il permet de tester la passerelle sans RSP ni bibliothèque SDRplay.

## Lancer la passerelle en mode factice

    cargo build --release --no-default-features
    ./target/release/sdr-universal --mock --port 2234 --mock-level -70

Options : `--mock` (ou `SDR_MOCK=1`), `--port N` (rtl_tcp ; le port de contrôle
est N+1), `--mock-level DBM` (ou `SDR_MOCK_LEVEL_DBM`, -75 par défaut), `--verbose`.
Le signal est du bruit : un client DAB s'y connecte, règle son gain et affiche un
niveau RF, mais ne trouvera aucun ensemble.

## Lancer les tests

    cargo test --no-default-features      # sans l'API SDRplay
    cargo test                            # avec, sur la machine du RSP (le RSP n'est pas utilisé)

- tests unitaires : tables de gain, bande passante, trame du port de contrôle, modèle du mock ;
- `tests/mock_e2e.rs` : la passerelle est lancée en `--mock` et pilotée comme AbracaDABra
  (en-tête RTL0, gain manuel et AGC, niveau RF estimé, changement de bande, surcharge,
  reconnexions, commandes invalides, arrêt par Ctrl+C).

Limites : le mock approche les atténuations LNA de la bande 60-420 MHz pour toutes
les bandes ; il valide la logique de la passerelle, pas le comportement RF réel du RSP.
Le backend SDRplay lui-même n'est pas exécuté par ces tests.
