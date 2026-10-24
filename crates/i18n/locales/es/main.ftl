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
sync-part-downloaded = { $count ->
    [one] 1 descargado ({ $bytes })
   *[other] { $count } descargados ({ $bytes })
}
sync-part-uploaded = { $count ->
    [one] 1 subido ({ $bytes })
   *[other] { $count } subidos ({ $bytes })
}
sync-part-moved = { $count ->
    [one] 1 movido
   *[other] { $count } movidos
}
sync-part-removed = { $count ->
    [one] 1 eliminado de este equipo
   *[other] { $count } eliminados de este equipo
}
sync-part-trashed = { $count ->
    [one] 1 enviado a la papelera de Drive
   *[other] { $count } enviados a la papelera de Drive
}
sync-part-copies = { $count ->
    [one] 1 copia en conflicto
   *[other] { $count } copias en conflicto
}
sync-part-nothing = nada cambió
sync-conflict-copy = Cambió aquí y en Google Drive; tu versión se conservó como { $path }
conflict-type-clash = Se dejó como está: un archivo y una carpeta tienen el mismo nombre
conflict-read-only = Tus cambios se quedan en este equipo: no puedes editar este archivo en Google Drive
conflict-cannot-delete = Restaurado: no puedes eliminar este elemento en Google Drive
sync-started = { $email }: sincronizando…
# Inserted in file names, e.g. "report (conflict copy 2026-10-23 14-30).pdf".
conflict-copy-label = copia en conflicto { $date }
sync-conflict = Se conservó la versión local, que cambió en este equipo: { $path }
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

notify-conflict-title = { $count ->
    [one] Un archivo cambió en dos lugares
   *[other] { $count } archivos cambiaron en dos lugares
}
notify-attention-title = { $count ->
    [one] 1 elemento necesita atención
   *[other] { $count } elementos necesitan atención
}
notify-problem-title = No se pudo sincronizar { $email }
notify-mount-failed-title = No se pudo abrir la carpeta de Google Drive de { $email }
notify-upload-failed-title = Aún no se pudo subir «{ $name }»
notify-and-more = { $list } y { $count } más
notify-open = Abrir

# strftime syntax: https://docs.rs/jiff/latest/jiff/fmt/strtime/
date-format = %d/%m/%Y %H:%M
trash-restored = “{ $name }” se restauró en { $location }.
trash-restored-unknown = “{ $name }” se restauró.
trash-location-unknown = una carpeta a la que ya no tienes acceso

stream-pending = { $count ->
    [one] 1 archivo guardado antes aún no estaba en Google Drive; subiéndolo.
   *[other] { $count } archivos guardados antes aún no estaban en Google Drive; subiéndolos.
}

kept-files-label = archivos conservados
mode-kept-files = Los archivos que solo existían en este equipo se movieron a { $path }.

offline-complete = { $total }, en este equipo
offline-progress = { $cached } de { $total } descargados

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
cli-sync-root-changed = Carpeta de sincronización cambiada a { $path }. Los archivos sincronizados se movieron allí.
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
cli-sync-confirm-deletions = Enviar a la papelera de Google Drive los archivos eliminados en este equipo, aunque sean muchos
cli-sync-restore-deletions = Volver a descargar los archivos eliminados en este equipo en lugar de enviarlos a la papelera de Google Drive
cli-trash-about = Ver y restaurar elementos de la papelera de Google Drive
cli-trash-list-about = Listar lo que hay en la papelera (se guarda 30 días)
cli-trash-restore-about = Devolver un elemento a donde estaba
cli-trash-item-id = Id del elemento, como lo muestra "trash list"
cli-trash-empty = La papelera está vacía.
cli-trash-entry = { $name }  (en { $location })  [{ $id }]
cli-stream-about = Mostrar todo el Google Drive de una cuenta en una carpeta, descargando los archivos solo cuando se abren
cli-stream-mountpoint = Carpeta donde mostrarlo (se crea si no existe; debe estar vacía)
cli-stream-mounted = Google Drive está disponible en { $path }. Pulsa Ctrl+C para detener.
cli-stream-unmounted = Carpeta de Google Drive cerrada.
cli-config-mode-about = Reflejar las carpetas seleccionadas, o transmitir todo el Drive (archivos descargados al abrirlos)
cli-config-mode-value = "mirror" (reflejar) o "stream" (transmitir)
cli-mode-stream = { $email } ahora transmite todo su Google Drive en su carpeta.
cli-mode-mirror = { $email } ahora refleja las carpetas seleccionadas. Elígelas con "nextgdrive sync select".
cli-stream-already-mounted = { $email } ya se transmite en su carpeta desde el Next GDrive en segundo plano.
cli-offline-about = Conservar archivos y carpetas transmitidos en este equipo, para usarlos sin internet
cli-offline-add-about = Hacer que un archivo o carpeta esté disponible sin conexión
cli-offline-remove-about = Dejar de conservar un archivo o carpeta sin conexión (sigue en Google Drive)
cli-offline-list-about = Listar lo que está disponible sin conexión
cli-offline-path = Ruta dentro de la carpeta de una cuenta transmitida, p. ej. ~/GoogleDrive/tu@gmail.com/Mi unidad/Docs
cli-offline-added = «{ $name }» se conservará en este equipo. Se descarga en segundo plano.
cli-offline-removed = «{ $name }» ya no se conserva sin conexión.
cli-offline-empty = No hay nada disponible sin conexión.
cli-offline-outside = { $path } no está dentro de la carpeta de una cuenta vinculada.
cli-offline-not-streamed = { $email } refleja sus carpetas, que siempre están en este equipo; el modo sin conexión es para cuentas transmitidas.

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
gui-sync-now = Sincronizar ahora
gui-sync-never = Aún no sincronizado en esta sesión
gui-syncing = Sincronizando…
gui-report-conflicts = { $count ->
    [one] 1 elemento necesita atención
   *[other] { $count } elementos necesitan atención
}
gui-report-copies = { $count ->
    [one] 1 archivo cambió en ambos lados y se conservó como copia en conflicto
   *[other] { $count } archivos cambiaron en ambos lados y se conservaron como copias en conflicto
}
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
gui-sync-root-note = Cada cuenta tiene su propia carpeta dentro de ella. Al cambiarla, los archivos sincronizados se mueven.
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
gui-deletions-title = { $count } archivos eliminados en este equipo
gui-deletions-body = ¿Enviarlos también a la papelera de Google Drive, donde pueden restaurarse durante 30 días? ¿O volver a descargarlos?
gui-deletions-restore = Restaurar archivos
gui-deletions-trash = Mover a la papelera
gui-sync-title = Sincronización
gui-shared-drives = Unidades compartidas
gui-shared-drives-description = Unidades que tu equipo u organización comparte contigo.
gui-notifications = Notificaciones
gui-notifications-subtitle = Avisarme de conflictos y problemas de sincronización
gui-trash = Papelera de Google Drive
gui-trash-subtitle = Restaurar elementos eliminados en los últimos 30 días
gui-trash-empty-title = La papelera está vacía
gui-trash-empty-description = Los elementos eliminados de Google Drive se quedan aquí 30 días.
gui-trash-restore = Restaurar
gui-trash-entry = En { $location }
gui-trash-entry-dated = En { $location } · eliminado el { $date }
gui-mode = Modo
gui-mode-mirror = Reflejar carpetas seleccionadas
gui-mode-stream = Transmitir todo el Drive
gui-mode-mirror-subtitle = Las carpetas elegidas se copian a este equipo y funcionan sin conexión
gui-mode-stream-subtitle = Se muestra todo; los archivos se descargan al abrirlos
gui-mode-stream-title = ¿Transmitir todo el Drive?
gui-mode-stream-body = La carpeta de la cuenta mostrará todo tu Google Drive, descargando los archivos solo cuando se abran. Las copias descargadas se eliminan de este equipo (siguen en Google Drive y vuelven a aparecer en la carpeta). Los archivos que solo existen en este equipo se mueven a una carpeta aparte, nunca se eliminan.
gui-mode-stream-action = Transmitir
gui-mode-mirror-title = ¿Reflejar carpetas seleccionadas?
gui-mode-mirror-body = La carpeta de la cuenta guardará copias de las carpetas que elijas, sincronizadas y disponibles sin conexión. Elígelas en Carpetas para sincronizar.
gui-mode-mirror-action = Reflejar
gui-mode-switched = Modo cambiado.
gui-offline = Disponible sin conexión
gui-offline-subtitle = Archivos y carpetas conservados en este equipo, para usarlos sin internet
gui-offline-description = Se descargan por completo y se mantienen actualizados. Lo demás solo se descarga al abrirlo.
gui-offline-empty = Nada todavía. Añade archivos o carpetas con los botones de arriba.
gui-offline-add-file = Añadir un archivo
gui-offline-add-folder = Añadir una carpeta
gui-offline-remove = Dejar de conservar sin conexión

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
error-local-folder-missing = No se encuentra la carpeta sincronizada { $path } (¿un disco desmontado?). No se cambió nada en Google Drive; la sincronización sigue cuando vuelva la carpeta.
error-mass-deletion = Se eliminaron { $count } archivos en este equipo. Confirma antes de que vayan a la papelera de Google Drive, o restáuralos.
error-folder-exists = Ya existe una carpeta en { $path }. Elige otra ubicación, o mueve o elimina esa carpeta primero.
error-mount = No se pudo mostrar Google Drive en { $path }: { $details }
error-pending-uploads = { $count ->
    [one] 1 archivo guardado en la carpeta transmitida aún no está en Google Drive. Vuelve a intentarlo cuando se haya subido.
   *[other] { $count } archivos guardados en la carpeta transmitida aún no están en Google Drive. Vuelve a intentarlo cuando se hayan subido.
}
error-mount-not-empty = La carpeta { $path } no está vacía, así que el Google Drive transmitido no se puede mostrar allí. Mueve su contenido a otro lugar.
