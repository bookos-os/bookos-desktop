# Política de seguridad

## Cómo avisar de una vulnerabilidad

Escribe a **report@bookos.es**. Por favor, **no abras un issue público** para un
fallo de seguridad: queda a la vista de todos antes de que exista un arreglo.

Cuéntanos, si puedes:

- Qué componente afecta (compositor, shell, pantalla de acceso, servicio del
  sistema…) y en qué versión.
- Cómo reproducirlo, con los pasos mínimos.
- Qué consigue quien lo explota: leer algo, ejecutar algo, saltarse el bloqueo…

## Qué esperar

- Acusamos recibo del aviso en un plazo de **7 días**.
- Te contamos si lo consideramos una vulnerabilidad y cómo vamos a abordarla.
- Publicamos el arreglo antes de dar detalles, y te citamos en la nota si lo
  quieres.

Esto lo mantiene un equipo pequeño: los plazos son una intención, no un
contrato.

## Qué cuenta especialmente

El escritorio hace de guardián de varias cosas, y un fallo ahí pesa más:

- **Pantalla de bloqueo y de acceso:** poder saltarse la contraseña o la huella,
  o que una ventana se dibuje por encima del bloqueo.
- **Lo que viene de otros procesos:** el `app_id`, el título o el contenido que
  declara un cliente acaban en el dock, en el buscador y en órdenes que se
  lanzan. Una inyección por ahí es una vulnerabilidad.
- **Servicios con privilegios:** `bookos-system` y `bookos-budsd` atienden
  peticiones por D-Bus.
- **Instalación:** `session/instalar.sh` escribe en `/usr` y `/etc`.

## Versiones con soporte

Solo la última versión publicada. BookOS está en desarrollo temprano y los
arreglos no se retroceden a versiones anteriores.
