#!/usr/bin/env bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

FILE="src/backend/sdrplay/mod.rs"

python3 - "$FILE" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()

marker = "    pub fn disconnect("

if "pub fn set_gain(" in text:
    print("set_gain() est déjà présent : aucune modification effectuée.")
    raise SystemExit(0)

pos = text.find(marker)
if pos < 0:
    raise SystemExit("ERREUR : impossible de trouver 'pub fn disconnect(' dans mod.rs")

method = r'''    pub fn set_gain(
        &mut self,
        gain_db: f32,
    ) -> Result<()> {
        if !self.connected {
            return Err(anyhow!(
                "SDRplay non connecté"
            ));
        }

        if !self.device_selected {
            return Err(anyhow!(
                "Aucun périphérique SDRplay sélectionné"
            ));
        }

        if !self.initialized {
            return Err(anyhow!(
                "RSP1B non initialisé"
            ));
        }

        if !gain_db.is_finite() {
            return Err(anyhow!(
                "Gain invalide : {} dB",
                gain_db
            ));
        }

        if gain_db < 0.0 || gain_db > 59.0 {
            return Err(anyhow!(
                "Gain hors plage : {:.1} dB (plage Core actuelle : 0..59 dB)",
                gain_db
            ));
        }

        let device =
            self.selected_device
                .ok_or_else(|| {
                    anyhow!(
                        "Périphérique SDRplay absent"
                    )
                })?;

        unsafe {
            let mut params:
                *mut SdrplayDeviceParams =
                std::ptr::null_mut();

            let result =
                sdrplay_api_GetDeviceParams(
                    device.dev,
                    &mut params,
                );

            if result != 0 {
                return Err(anyhow!(
                    "GetDeviceParams() a échoué : {}",
                    result
                ));
            }

            if params.is_null() {
                return Err(anyhow!(
                    "GetDeviceParams() retourne NULL"
                ));
            }

            if (*params)
                .rx_channel_a
                .is_null()
            {
                return Err(anyhow!(
                    "rxChannelA est NULL"
                ));
            }

            let rx =
                &mut *(*params)
                    .rx_channel_a;

            /*
             * SDRplay appelle cette valeur "gRdB" :
             * gain reduction en dB.
             *
             * Pour cette première étape, la valeur du
             * Core est appliquée directement à gRdB.
             * Le réglage LNA sera traité séparément dans
             * l'étape de gestion complète du gain.
             */
            rx.tuner_params
                .gain
                .gr_db =
                gain_db.round() as c_int;

            const UPDATE_TUNER_GR: c_int =
                0x00008000;

            const TUNER_A: c_int = 1;
            const EXT1_NONE: c_int = 0;

            let result =
                sdrplay_api_Update(
                    device.dev,
                    TUNER_A,
                    UPDATE_TUNER_GR,
                    EXT1_NONE,
                );

            if result != 0 {
                return Err(anyhow!(
                    "sdrplay_api_Update(GR) a échoué : {}",
                    result
                ));
            }

            println!(
                "Gain reduction RSP1B réglé à {} dB",
                gain_db.round() as c_int
            );
        }

        Ok(())
    }

'''

text = text[:pos] + method + text[pos:]
path.write_text(text)
print(f"Modification appliquée à {path}")
PY

cargo fmt -- src/backend/sdrplay/mod.rs

echo
 echo "Vérification :"
grep -n -A8 "pub fn set_gain" "$FILE"
