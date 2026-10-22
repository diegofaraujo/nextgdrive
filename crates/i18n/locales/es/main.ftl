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
sync-summary = { $email }: { $downloaded } descargados ({ $bytes }), { $moved } movidos, { $removed } eliminados
sync-conflict = Se conservó la versión local, que cambió en este equipo: { $path }
sync-skipped-shortcut = Acceso directo de Drive omitido: { $path }
sync-skipped-locked = Omitido, el propietario no permite descargarlo: { $path }
sync-failed = Falló: { $path }: { $error }

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
cli-sync-progress = { $done } de { $total } archivos, { $bytes_done } de { $bytes_total }
cli-sync-nothing-selected = { $email }: nada seleccionado. Elige carpetas con "nextgdrive sync select".

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
