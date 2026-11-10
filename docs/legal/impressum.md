# Impressum

Angaben gemäß § 5 Digitale-Dienste-Gesetz (DDG).

## Diensteanbieter

Michael Protogerakis
Lindener Str. 156
52146 Würselen
Deutschland

## Kontakt

E-Mail: <span id="kontakt-mail"><noscript>proto (at) zqel (punkt) org</noscript></span>

<script>
(function () {
  // Die beiden Hälften stehen nie verbunden im Quelltext; das "@" entsteht
  // erst hier. Das hält Adresssammler ab, die das HTML nach "@" durchsuchen
  // — eine Hürde, kein Schutz: wer JavaScript ausführt, bekommt sie.
  var teile = ["proto", "zqel.org"];
  var ziel = document.getElementById("kontakt-mail");
  if (!ziel) return;
  var adresse = teile[0] + String.fromCharCode(64) + teile[1];
  var a = document.createElement("a");
  a.href = "mailto:" + adresse;
  a.textContent = adresse;
  ziel.textContent = "";
  ziel.appendChild(a);
})();
</script>
## Verantwortlich für den Inhalt

Michael Protogerakis, Anschrift wie oben.

## Haftung für Links

Diese Website verweist auf externe Seiten, auf deren Inhalte kein Einfluss
besteht. Für diese fremden Inhalte wird keine Gewähr übernommen;
verantwortlich ist stets deren jeweiliger Anbieter oder Betreiber. Die
verlinkten Seiten wurden zum Zeitpunkt der Verlinkung auf mögliche
Rechtsverstöße geprüft; eine dauerhafte inhaltliche Kontrolle ist ohne
konkreten Anhaltspunkt nicht zumutbar. Bei Bekanntwerden von
Rechtsverletzungen werden entsprechende Links entfernt.

## Urheberrecht

Die auf dieser Website veröffentlichten Inhalte und Werke unterliegen dem
deutschen Urheberrecht. Der Quellcode von nomospice steht unter der
AGPL-3.0-or-later, wie im [Repository](https://github.com/mprotogerakis/nomospice)
angegeben; diese geht dieser Angabe für den Code vor.
