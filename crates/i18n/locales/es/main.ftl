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
