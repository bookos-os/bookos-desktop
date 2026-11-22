# API de la isla dinámica

> Documento de referencia para la web de BookUI. Describe la versión 0.2.2.
> Pensado para que se convierta en una página: cada sección `##` es un apartado.

La **isla** es la píldora del centro del panel. Enseña lo que está pasando ahora
en una app —una canción, un temporizador, una copia de archivos— sin abrir su
ventana. Al pulsarla se despliega en una tarjeta con controles, y lo que el
usuario toca vuelve a la app como una orden.

Una app no dibuja nada: **publica un estado** (unos pocos campos en JSON) y la
isla decide cómo pintarlo según el **tipo** de actividad.

---

## Antes de empezar: quién puede publicar

La isla no es una segunda bandeja de notificaciones. Hoy solo pueden usarla las
apps de BookOS, cada una con su tipo:

| `app_id`                   | Tipo          | Programa que debe llamar                      |
|----------------------------|---------------|-----------------------------------------------|
| `com.bookos.player`        | `player`      | `bookos-player`                               |
| `com.bookos.clock`         | `timer`       | `bookos-clock`                                |
| `com.bookos.voicerecorder` | `recorder`    | `bookos-voicerecorder`                        |
| `com.bookos.settings`      | `transfer`    | `bookos-settings`, `bookos-bookshare`         |
| `com.bookos.explorer`      | `transfer`    | `bookos-explorer`                             |
| `com.bookos.calendar`      | `calendar`    | `bookos-calendar`                             |
| `com.bookos.systemmonitor` | `monitor`     | `bookos-system-monitor`                       |
| `com.bookos.buds`          | `auriculares` | `bookos-budsd`                                |
| `com.bookos.assistant`     | `asistente`   | `bookos-assistant`                            |

El escritorio comprueba **qué programa** hace la llamada (por su ejecutable; en
scripts de Python, por el nombre del script). No basta con decir que eres el
Player: tienes que serlo.

---

## Inicio rápido

Publicar es una llamada D-Bus en el bus de sesión. La actividad sigue viva
mientras tu conexión al bus siga abierta: **si tu proceso sale, la isla la
retira sola**.

### Python

```python
import json
from gi.repository import Gio, GLib

bus = Gio.bus_get_sync(Gio.BusType.SESSION)

def publicar(estado: dict) -> None:
    bus.call_sync(
        "org.bookos.Desktop", "/org/bookos/Desktop", "org.bookos.Desktop",
        "PublishActivity",
        GLib.Variant("(sss)", ("com.bookos.clock", "timer", json.dumps(estado))),
        None, Gio.DBusCallFlags.NONE, 3000, None,
    )

publicar({"activo": True, "titulo": "Pasta", "duracion_ms": 600_000, "restante_ms": 45_000})
GLib.MainLoop().run()  # seguir vivo: al salir, la actividad desaparece
```

### Rust (zbus)

```rust
let conexion = zbus::Connection::session().await?;
let estado = serde_json::json!({
    "activo": true,
    "titulo": "Pasta",
    "duracion_ms": 600_000,
    "restante_ms": 45_000,
});
let _: bool = conexion
    .call_method(
        Some("org.bookos.Desktop"),
        "/org/bookos/Desktop",
        Some("org.bookos.Desktop"),
        "PublishActivity",
        &("com.bookos.clock", "timer", estado.to_string()),
    )
    .await?
    .body()
    .deserialize()?;
```

> **No pruebes con `busctl`.** `busctl call` publica y sale en el acto, y la
> isla retira la actividad en cuanto su dueño se va del bus. Parecerá que no
> funciona.

---

## Métodos

Servicio `org.bookos.Desktop`, ruta `/org/bookos/Desktop`, interfaz
`org.bookos.Desktop`.

### `PublishActivity(app_id: s, kind: s, state_json: s) → b`

Crea la actividad o actualiza la que ya tenías. Llámalo cada vez que cambie
algo (una vez por segundo es lo normal para un reproductor o un temporizador).

- `app_id`: el de tu app, de la tabla de arriba.
- `kind`: el tipo. Puede ir vacío (`""`) y se usa el de tu app.
- `state_json`: el estado, ver [El estado](#el-estado).

Devuelve `true` si se publicó. **Si se rechaza, la llamada falla con un error
D-Bus que dice el motivo** (ver [Errores](#errores)).

### `CloseActivity(app_id: s) → b`

Retira tu actividad. Solo la cierra quien la publicó: si hay dos instancias de
tu app, una no puede borrar la de la otra.

### Señal `ActivityAction(app_id: s, action: s, value: s)`

Lo que el usuario pulsa en la tarjeta. Escúchala y filtra por tu `app_id`. Las
acciones de cada tipo están en [Tipos de actividad](#tipos-de-actividad).

```python
def al_recibir(_con, _remitente, _ruta, _interfaz, _senal, args):
    app_id, accion, valor = args.unpack()
    if app_id == "com.bookos.clock" and accion == "pause":
        pausar()

bus.signal_subscribe("org.bookos.Desktop", "org.bookos.Desktop", "ActivityAction",
                     "/org/bookos/Desktop", None, Gio.DBusSignalFlags.NONE, al_recibir)
```

---

## Errores

Todos llegan como `org.freedesktop.DBus.Error.Failed` con un mensaje en
castellano que dice exactamente qué pasa:

| Mensaje                                                         | Qué significa                                                          |
|-----------------------------------------------------------------|------------------------------------------------------------------------|
| `«com.x» no está en la lista de apps que pueden usar la isla`   | El `app_id` no está en la tabla.                                        |
| `«com.bookos.player» no puede publicar actividades de tipo «timer»` | El `app_id` existe, pero no con ese tipo.                          |
| `el programa que llama no es el de «com.bookos.player»`         | Tu ejecutable no es el de ese `app_id`.                                 |
| `«…»: estado no válido: invalid type: integer 5, expected a string at line 1 column 12` | El JSON está mal: el motivo y dónde, tal cual lo dice el parser. |
| `«…»: el estado pasa de 8 MB`                                   | Demasiado grande; casi siempre, carátulas enormes.                      |
| `fase del asistente desconocida: «…»`                           | Solo Zenith: `fase` no es una de las válidas.                           |
| `el compositor no está escuchando`                              | El escritorio se está cerrando o reiniciando. Reintenta más tarde.      |

---

## El estado

Un objeto JSON. **Todos los campos son opcionales**: lo que no mandes toma su
valor por defecto, y cada tipo usa solo los que le sirven.

| Campo          | Tipo              | Por defecto | Para qué                                                                 |
|----------------|-------------------|-------------|--------------------------------------------------------------------------|
| `activo`       | bool              | `false`     | Está pasando algo ahora (suena, cuenta, graba).                          |
| `pausado`      | bool              | `false`     | En pausa.                                                                |
| `titulo`       | texto             | `""`        | La línea principal. Máx. 120 caracteres; lo demás se corta.              |
| `subtitulo`    | texto             | `""`        | La segunda línea. Máx. 160.                                              |
| `posicion_ms`  | entero            | `0`         | Por dónde va (reproductor, grabadora).                                   |
| `duracion_ms`  | entero ≥ 0        | `0`         | Lo que dura en total.                                                    |
| `restante_ms`  | entero            | `0`         | Lo que falta. **Negativo** tiene significado propio según el tipo.       |
| `nivel`        | número 0–1        | `0`         | Progreso, o intensidad (la onda de la grabadora, la voz de Zenith).      |
| `volumen`      | entero 0–100      | `100`       | Volumen del reproductor.                                                 |
| `aleatorio`    | bool              | `false`     | Botón de aleatorio encendido.                                            |
| `repetir`      | bool              | `false`     | Repetición activada.                                                     |
| `anc`          | entero 0–3 o nada | nada        | Modo de ruido de los auriculares.                                        |
| `portada`      | texto             | `""`        | Imagen como `data:image/png;base64,…` (o jpeg). Vacía = la de antes.     |
| `fase`         | texto             | `""`        | Solo el asistente.                                                       |
| `cola`         | lista             | `[]`        | Solo el reproductor. Máx. 50 elementos.                                  |
| `metricas`     | lista             | `[]`        | Monitor y auriculares. Máx. 6.                                           |

Un campo con el **tipo equivocado** (un número donde va texto) rechaza la
publicación entera con el motivo; un campo **desconocido** se ignora.

### Elemento de `cola`

| Campo         | Tipo   | Para qué                                              |
|---------------|--------|-------------------------------------------------------|
| `id`          | texto  | Identificador tuyo; vuelve en las acciones.           |
| `titulo`      | texto  | Máx. 120.                                             |
| `artista`     | texto  | Máx. 120.                                             |
| `duracion_ms` | entero |                                                       |
| `favorita`    | bool   |                                                       |
| `actual`      | bool   | La que suena.                                         |
| `portada`     | texto  | Como `portada`. Vacía = la que ya tenía ese `id`.     |

### Elemento de `metricas`

| Campo      | Tipo       | Para qué                                                        |
|------------|------------|-----------------------------------------------------------------|
| `etiqueta` | texto      | Máx. 24. «CPU», «Izquierdo»…                                    |
| `valor`    | número 0–1 | La barra.                                                       |
| `texto`    | texto      | Máx. 16. Lo que se lee al lado: «42 %», «3,1 GB».               |
| `critica`  | bool       | Pintarla como aviso. Lo decides tú: la isla no sabe qué es grave. |

### Imágenes: mándalas solo cuando cambien

Una carátula en base64 pesa cientos de KB. Mándala cuando cambie y deja
`portada` vacía en las demás publicaciones: la isla conserva la última. Lo mismo
con las de la cola, por `id`.

---

## Tipos de actividad

### `player` — reproductor

Píldora con carátula y onda; tarjeta con controles, barra de progreso, volumen
y la cola.

Campos: `activo`, `pausado`, `titulo` (canción), `subtitulo` (artista),
`posicion_ms`, `duracion_ms`, `volumen`, `aleatorio`, `repetir`, `portada`,
`cola`.

| Acción       | `value`                       |
|--------------|-------------------------------|
| `play-pause` |                               |
| `previous`   |                               |
| `next`       |                               |
| `shuffle`    |                               |
| `seek`       | Posición de 0 a 1, `"0.4210"` |
| `volume`     | 0–100                         |
| `favorite`   | `id` del elemento de la cola  |
| `queue-goto` | `id` del elemento de la cola  |

La isla cambia el icono de reproducir y el volumen **en el acto**, sin esperar
tu respuesta. Tu siguiente publicación deja el valor real.

### `timer` — temporizador

Campos: `titulo`, `duracion_ms`, `restante_ms`, `pausado`.
**`restante_ms` negativo = el tiempo ya se acabó** («Tiempo acabado»).

Solo aparece en la isla cuando quedan 60 s o menos, salvo que el usuario active
*Temporizador siempre visible* en Ajustes.

| Acción   | Cuándo                         |
|----------|--------------------------------|
| `pause`  | En marcha                      |
| `resume` | En pausa                       |
| `snooze` | Ya acabado (`restante_ms` < 0) |
| `stop`   | Siempre                        |

### `recorder` — grabadora

Campos: `pausado` («Grabando» / «En pausa»), `posicion_ms` (tiempo grabado),
`nivel` (la onda, 0–1).

Acciones: `pause`, `resume`, `stop`.

### `transfer` — copia o envío de archivos

Campos: `titulo` (archivo), `subtitulo` (p. ej. «Copiando a Documentos · 12 MB/s»),
`nivel` (progreso 0–1), `restante_ms` (**negativo = no se sabe**, y no se
enseña «Quedan…»).

Acción: `cancel`.

### `calendar` — próximo evento

Campos: `titulo` (evento), `subtitulo` (lugar u hora), `restante_ms` hasta que
empiece (**negativo = ya ha empezado**, «En curso»).

Acción: `dismiss`.

### `monitor` — rendimiento

Campos: `metricas` (caben 5 filas a la vista). Solo informa: no tiene acciones.
Tiene la prioridad más baja: nunca le quita la isla a otra actividad.

### `auriculares` — auriculares Bluetooth

Campos: `titulo` (nombre), `metricas` (hasta 3: baterías), `anc`:
`0` desactivada, `1` cancelación de ruido, `2` ambiente.
Sin `anc`, la tarjeta ofrece abrir los ajustes.

| Acción | `value`                 |
|--------|-------------------------|
| `anc`  | Modo elegido, `0`/`1`/`2` |
| `open` |                         |

### `asistente` — Zenith

Exclusivo del asistente. `fase` obligatoria: `escuchando`, `pensando`,
`respuesta`, `hecho`, `error` o `minimizado`. `subtitulo` lleva el texto de la
respuesta; `nivel`, la intensidad de la voz.

Acciones: `finish`, `minimize`, `expand`, `cancel`, `dismiss`, `retry`.

---

## Cómo decide la isla qué enseñar

Con varias actividades a la vez, la píldora muestra una y las demás quedan en
una columna al desplegarla. El usuario las recorre con la rueda o con dos dedos.

1. **Prioridad por tipo**, de más a menos: asistente hablando, grabadora,
   evento de calendario, temporizador acabado, temporizador, transferencia,
   auriculares, reproductor, asistente minimizado, monitor.
2. **La que el usuario elige** con la rueda queda como principal hasta que
   termine.
3. **Tu actividad se oculta mientras tu ventana tiene el foco**: si ya estás
   viendo la app, no hace falta repetirlo arriba.
4. El usuario puede ocultar las actividades de una app desde Ajustes, o
   apagarlas todas. El asistente nunca se oculta.

---

## Buenas prácticas

- **Publica cuando cambie algo**, no en bucle. Un reproductor: una vez por
  segundo. Una transferencia: cada pocos porcentajes.
- **Cierra al terminar** con `CloseActivity`. Si tu proceso muere, la isla la
  retira sola, pero una actividad que ya no pinta nada no debe quedarse.
- **Comprueba el error** de `PublishActivity` mientras desarrollas: el mensaje
  dice qué falla. En producción, un fallo de la isla no debe romper tu app.
- **Textos cortos.** La píldora es estrecha; el título se recorta.
