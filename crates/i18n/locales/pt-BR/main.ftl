app-name = Next GDrive

## Shared

# Used to format sizes such as "1.5 GB".
decimal-separator = ,
account-removed = Conta { $email } desvinculada. Os arquivos locais foram mantidos.
account-added = Conta { $email } vinculada.
quota-used = { $used } de { $total } usados ({ $free } livres)
quota-used-unlimited = { $used } usados
auth-callback-success = Login concluído. Você pode fechar esta aba e voltar ao Next GDrive.
auth-callback-failure = O login não foi concluído. Volte ao Next GDrive e tente novamente.

## CLI

cli-about = Sincronize suas contas do Google Drive com pastas locais.
cli-heading-usage = Uso:
cli-heading-commands = Comandos
cli-heading-arguments = Argumentos
cli-heading-options = Opções
cli-help = Mostrar ajuda
cli-version = Mostrar versão
cli-account-about = Gerenciar contas Google vinculadas
cli-account-list-about = Listar contas vinculadas
cli-account-remove-about = Desvincular uma conta (os arquivos locais são mantidos)
cli-account-remove-email = E-mail da conta a desvincular
cli-account-add-about = Vincular uma conta Google (abre o navegador para fazer login)
cli-account-status-about = Mostrar o uso de armazenamento das contas vinculadas
cli-account-status-email = Apenas esta conta (padrão: todas)
cli-account-add-open-url = Abra este endereço no navegador para fazer login:
cli-account-add-waiting = Aguardando o login…
cli-account-list-empty = Nenhuma conta vinculada.

## GUI

gui-accounts-title = Contas
gui-no-accounts-title = Nenhuma conta
gui-no-accounts-description = As contas Google vinculadas aparecerão aqui.
gui-load-error-title = Não foi possível carregar as contas
gui-remove-account = Desvincular conta
gui-remove-confirm-title = Desvincular { $email }?
gui-remove-confirm-body = A conta deixará de ser sincronizada. Os arquivos que já estão neste computador serão mantidos.
gui-cancel = Cancelar
gui-remove = Desvincular
gui-add-account = Adicionar conta
gui-add-waiting-title = Entrar com o Google
gui-add-waiting-body = Continue no navegador. Se ele não abriu, use o link abaixo.
gui-open-browser = Abrir página de login
gui-quota-loading = Verificando armazenamento…
gui-quota-error = Não foi possível verificar o armazenamento

## Errors

error-account-not-found = Nenhuma conta com o e-mail { $email }.
error-invalid-email = Endereço de e-mail inválido: { $email }
error-storage = Não foi possível acessar os dados armazenados: { $details }
error-no-config-dir = Não foi possível determinar o diretório de configuração ($HOME está definido?).
error-missing-credentials = Não há credenciais salvas para { $email }. Adicione a conta novamente.
error-auth = Falha no login: { $details }
error-credentials = Não foi possível acessar o chaveiro do sistema: { $details }
error-drive = Falha na requisição ao Google Drive: { $details }
error-oauth-client-missing = O cliente OAuth do Google não está configurado. Salve o arquivo JSON baixado do Google Cloud Console como { $path }, ou defina { $id_var } e { $secret_var }.
error-oauth-client-unreadable = Não foi possível ler o arquivo do cliente OAuth { $path }: { $details }
