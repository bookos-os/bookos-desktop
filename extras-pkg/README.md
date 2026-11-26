# bookos-extras

Paquete opcional de navegación táctil estilo Android y cortina de notificaciones ampliada para BookOS Desktop.

**No viene instalado ni activado por defecto** con `bookos-desktop`.

## Gestos incluidos

- **Borde superior → abajo (en 2 pasos como Android):**
  - **1er arrastre (si el panel está oculto):** solo muestra el panel superior para poder tocar cualquiera de sus iconos.
  - **2º arrastre (con el panel ya visible):**
    - **Izquierda:** abre el menú de BookOS (`Meta+Z`).
    - **Centro:** despliega la cortina ampliada de notificaciones.
    - **Derecha:** abre el Centro de control (`Meta+X`).
- **Abajo → arriba:** un recorrido corto abre el dock. Si el dedo sigue
  subiendo, el mismo gesto abre Aplicaciones recientes. Con Recientes
  abierto, deslizar de nuevo hacia arriba lleva al escritorio.
- **Abajo horizontal:** cambio continuo entre escritorios.
- **Borde izquierdo hacia dentro:** gesto «Atrás» (`XF86Back`) con cápsula flotante fluida.
- **Borde derecho hacia dentro:** gesto «Adelante» (`XF86Forward`) con cápsula flotante fluida.

## Activar / desactivar

Una vez instalado el paquete, puedes activar o desactivar la navegación táctil en cualquier momento:

```bash
bookos-extras status
bookos-extras enable
bookos-extras disable
bookos-extras toggle
```

También hay un botón táctil **Gestos: ON / OFF** en la cabecera de la cortina de notificaciones.

## Instalación

- **Fedora / RPM:** `sudo dnf install ./bookos-extras-*.rpm`
- **Debian / Ubuntu / DEB:** `sudo apt install ./bookos-extras_*.deb`
