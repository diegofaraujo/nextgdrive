app-name = Next GDrive

## Shared

# Used to format sizes such as "1.5 GB".
decimal-separator = ,
account-removed = Cuenta { $email } desvinculada. Sus archivos locales se conservaron.
account-added = Cuenta { $email } vinculada.
quota-used = { $used } de { $total } usados ({ $free } libres)
quota-used-unlimited = { $used } usados
auth-callback-success = Has iniciado sesión. Puedes cerrar esta pestaña y volver a Next GDrive.
auth-callback-failure = No se completó el inicio de sesión. Vuelve a Next GDrive e inténtalo de nuevo.

# Folders created inside each account folder. Chosen once, on first use.
folder-my-drive = Mi unidad
folder-shared-drives = Unidades compartidas
sync-progress = { $done } de { $total } archivos, { $bytes_done } de { $bytes_total }
sync-started = { $email }: sincronizando…
sync-summary = { $email }: { $downloaded } descargados ({ $bytes }), { $moved } movidos, { $removed } eliminados
sync-conflict = Se conservó la versión local, que cambió en este equipo: { $path }
sync-conflict-exists = Se conservó el archivo local existente, que es distinto de Drive y no lo creó Next GDrive: { $path }
sync-skipped-shortcut = Acceso directo de Drive omitido: { $path }
sync-skipped-locked = Omitido, el propietario no permite descargarlo: { $path }
sync-failed = Falló: { $path }: { $error }

status-up-to-date = Actualizado
status-checked-ago = { $minutes ->
    [0] comprobado ahora
    [one] comprobado hace 1 minuto
   *[other] comprobado hace { $minutes } minutos
}
status-starting = Iniciando…
status-checking = Comprobando { $email }…
status-syncing = Sincronizando { $email }: { $done } de { $total } archivos
status-paused = Sincronización en pausa
status-problem = Problema con { $email }: { $message }

## CLI

cli-about = Sincroniza tus cuentas de Google Drive con carpetas locales.
cli-heading-usage = Uso:
cli-heading-commands = Comandos
cli-heading-arguments = Argumentos
cli-heading-options = Opciones
cli-help = Mostrar ayuda
cli-version = Mostrar versión
cli-account-about = Administrar cuentas de Google vinculadas
cli-account-list-about = Listar cuentas vinculadas
cli-account-remove-about = Desvincular una cuenta (los archivos locales se conservan)
cli-account-remove-email = Correo de la cuenta a desvincular
cli-account-add-about = Vincular una cuenta de Google (abre el navegador para iniciar sesión)
cli-account-status-about = Mostrar el uso de almacenamiento de las cuentas vinculadas
cli-account-status-email = Solo esta cuenta (predeterminado: todas)
cli-account-add-open-url = Abre esta dirección en tu navegador para iniciar sesión:
cli-account-add-waiting = Esperando a que inicies sesión…
cli-account-list-empty = No hay cuentas vinculadas.
cli-config-about = Mostrar o cambiar la configuración
cli-config-show-about = Mostrar la configuración actual
cli-config-sync-root-about = Cambiar la carpeta que contiene las cuentas sincronizadas
cli-config-sync-root-path = Nueva carpeta de sincronización
cli-sync-about = Elegir qué sincronizar
cli-sync-folders-about = Listar las unidades, o las subcarpetas de una carpeta, marcando las seleccionadas con [x]
cli-sync-folders-folder = Id de la carpeta (predeterminado: listar las unidades de la cuenta)
cli-sync-select-about = Sincronizar una carpeta y todo su contenido
cli-sync-deselect-about = Dejar de sincronizar una carpeta
cli-sync-folder-id = Id de la carpeta o unidad, como lo muestra "sync folders"
cli-account-email = Correo de la cuenta
cli-settings-sync-root = Carpeta de sincronización: { $path }
cli-settings-selection = { $email }: { $count ->
    [0] nada seleccionado
    [one] 1 carpeta seleccionada
   *[other] { $count } carpetas seleccionadas
}
cli-sync-root-changed = Carpeta de sincronización cambiada a { $path }. Los archivos ya sincronizados no se movieron.
cli-folder-selected = Carpeta seleccionada. Se descargará en la próxima sincronización.
cli-folder-deselected = Carpeta deseleccionada. Se conservaron las copias locales y ya no se actualizarán.
cli-no-folders = No hay subcarpetas.
cli-sync-run-about = Descargar las carpetas seleccionadas de una o de todas las cuentas
cli-sync-run-email = Solo esta cuenta (predeterminado: todas)
cli-sync-nothing-selected = { $email }: nada seleccionado. Elige carpetas con "nextgdrive sync select".
cli-sync-keep-local = Conservar los archivos descargados como archivos normales, sin actualizaciones
cli-sync-remove-local = Eliminar los archivos descargados de este equipo (siguen en Google Drive)
cli-folder-deselected-removed = Carpeta deseleccionada. { $removed ->
    [one] 1 elemento eliminado de este equipo.
   *[other] { $removed } elementos eliminados de este equipo.
}
cli-daemon-about = Mantener las cuentas sincronizadas en primer plano, sin interfaz gráfica
cli-daemon-started = Next GDrive está en ejecución y revisa Google Drive cada { $secs } segundos. Pulsa Ctrl+C para detenerlo.
cli-daemon-stopped = Next GDrive se detuvo.
cli-status-about = Mostrar qué está haciendo Next GDrive en segundo plano
cli-status-not-running = Next GDrive no se está ejecutando en segundo plano. Abre la aplicación o ejecuta "nextgdrive daemon".
cli-sync-requested = Se pidió al Next GDrive en segundo plano que sincronice ahora.
cli-daemon-already-running = Next GDrive ya se está ejecutando en segundo plano (la aplicación en la bandeja del sistema u otro "nextgdrive daemon").

## GUI

gui-accounts-title = Cuentas
gui-no-accounts-title = No hay cuentas
gui-no-accounts-description = Las cuentas de Google vinculadas aparecerán aquí.
gui-load-error-title = No se pudieron cargar las cuentas
gui-remove-account = Desvincular cuenta
gui-remove-confirm-title = ¿Desvincular { $email }?
gui-remove-confirm-body = La cuenta dejará de sincronizarse. Los archivos que ya están en este equipo se conservan.
gui-cancel = Cancelar
gui-remove = Desvincular
gui-add-account = Añadir cuenta
gui-add-waiting-title = Iniciar sesión con Google
gui-add-waiting-body = Continúa en tu navegador. Si no se abrió, usa el enlace de abajo.
gui-open-browser = Abrir página de inicio de sesión
gui-quota-loading = Comprobando almacenamiento…
gui-quota-error = No se pudo comprobar el almacenamiento
gui-main-menu = Menú principal
gui-preferences = Preferencias
gui-storage = Almacenamiento
gui-sync-group = Sincronización
gui-folders = Carpetas para sincronizar
gui-folders-count = { $count ->
    [0] Nada seleccionado
    [one] 1 carpeta seleccionada
   *[other] { $count } carpetas seleccionadas
}
gui-folders-description = Todo lo que esté dentro de una carpeta seleccionada se descarga en este equipo.
gui-folders-inherited = Sincronizada porque una carpeta superior está seleccionada.
gui-no-subfolders = No hay subcarpetas
gui-sync-title = Descarga
gui-sync-now = Sincronizar ahora
gui-sync-never = Aún no sincronizado en esta sesión
gui-syncing = Sincronizando…
gui-sync-result = { $downloaded } descargados ({ $bytes }), { $moved } movidos, { $removed } eliminados
gui-report-conflicts = { $count ->
    [one] 1 archivo local no se reemplazó
   *[other] { $count } archivos locales no se reemplazaron
}
gui-conflict-changed = Cambió en este equipo
gui-conflict-exists = Ya había un archivo distinto aquí
gui-report-failures = { $count ->
    [one] 1 elemento falló
   *[other] { $count } elementos fallaron
}
gui-report-skipped = { $count ->
    [one] 1 elemento omitido
   *[other] { $count } elementos omitidos
}
gui-skipped-shortcut = Acceso directo de Drive
gui-skipped-locked = El propietario no permite descargarlo
gui-open-folder = Abrir carpeta
gui-sync-root = Carpeta de sincronización
gui-sync-root-change = Cambiar…
gui-sync-root-note = Cada cuenta tiene su propia carpeta dentro de ella. Los archivos ya sincronizados se quedan donde están; la próxima sincronización descarga en la nueva carpeta.
gui-deselect-title = ¿Dejar de sincronizar «{ $name }»?
gui-deselect-body = Los archivos descargados pueden eliminarse de este equipo, y seguirán disponibles en Google Drive, o conservarse como archivos normales que ya no se actualizarán. Los archivos que cambiaste en este equipo siempre se conservan.
gui-deselect-keep = Conservar archivos
gui-deselect-remove = Eliminar archivos
gui-deselect-removed = { $removed ->
    [one] 1 elemento eliminado de este equipo
   *[other] { $removed } elementos eliminados de este equipo
}
gui-deselect-kept = { $count ->
    [one] 1 archivo conservado porque cambió en este equipo
   *[other] { $count } archivos conservados porque cambiaron en este equipo
}
gui-deselect-kept-files = Carpeta deseleccionada. Sus archivos se conservaron y ya no se actualizarán.
gui-background-group = Segundo plano
gui-autostart = Iniciar con la sesión
gui-autostart-subtitle = Mantener los archivos sincronizados desde la bandeja del sistema, sin abrir la ventana
gui-host-elsewhere = Otro proceso de Next GDrive (como "nextgdrive daemon") ya está sincronizando en segundo plano.
tray-open = Abrir Next GDrive
tray-sync-now = Sincronizar ahora
tray-pause = Pausar sincronización
tray-resume = Reanudar sincronización
tray-quit = Salir

## Errors

error-account-not-found = No hay ninguna cuenta con el correo { $email }.
error-invalid-email = Dirección de correo no válida: { $email }
error-storage = No se pudo acceder a los datos almacenados: { $details }
error-no-config-dir = No se pudo determinar el directorio de configuración (¿está definido $HOME?).
error-missing-credentials = No hay credenciales guardadas para { $email }. Vuelve a añadir la cuenta.
error-auth = Error al iniciar sesión: { $details }
error-credentials = No se pudo acceder al llavero del sistema: { $details }
error-drive = Falló la solicitud a Google Drive: { $details }
error-oauth-client-missing = El cliente OAuth de Google no está configurado. Guarda el archivo JSON descargado de Google Cloud Console como { $path }, o define { $id_var } y { $secret_var }.
error-oauth-client-unreadable = No se pudo leer el archivo del cliente OAuth { $path }: { $details }
error-invalid-sync-root = La carpeta de sincronización debe ser una ruta absoluta: { $path }
error-not-a-folder = No es una carpeta: { $id }
error-sync-in-progress = Esta cuenta ya se está sincronizando, en este u otro proceso de Next GDrive.
error-dbus = No se pudo usar el bus de la sesión (D-Bus): { $details }
