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

# Folders created inside each account folder. Chosen once, on first use.
folder-my-drive = Meu Drive
folder-shared-drives = Drives compartilhados
sync-progress = { $done } de { $total } arquivos, { $bytes_done } de { $bytes_total }
sync-started = { $email }: sincronizando…
sync-summary = { $email }: { $downloaded } baixados ({ $bytes }), { $moved } movidos, { $removed } removidos
sync-conflict = Mantida a versão local, que foi alterada neste computador: { $path }
sync-conflict-exists = Mantido o arquivo local existente, que é diferente do Drive e não foi criado pelo Next GDrive: { $path }
sync-skipped-shortcut = Atalho do Drive ignorado: { $path }
sync-skipped-locked = Ignorado, o proprietário não permite download: { $path }
sync-failed = Falhou: { $path }: { $error }

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
cli-config-about = Mostrar ou alterar configurações
cli-config-show-about = Mostrar as configurações atuais
cli-config-sync-root-about = Alterar a pasta que contém as contas sincronizadas
cli-config-sync-root-path = Nova pasta de sincronização
cli-sync-about = Escolher o que sincronizar
cli-sync-folders-about = Listar os drives, ou as subpastas de uma pasta, marcando as selecionadas com [x]
cli-sync-folders-folder = Id da pasta (padrão: listar os drives da conta)
cli-sync-select-about = Sincronizar uma pasta e tudo o que há dentro dela
cli-sync-deselect-about = Parar de sincronizar uma pasta
cli-sync-folder-id = Id da pasta ou do drive, como mostrado por "sync folders"
cli-account-email = E-mail da conta
cli-settings-sync-root = Pasta de sincronização: { $path }
cli-settings-selection = { $email }: { $count ->
    [0] nada selecionado
    [one] 1 pasta selecionada
   *[other] { $count } pastas selecionadas
}
cli-sync-root-changed = Pasta de sincronização alterada para { $path }. Os arquivos já sincronizados não foram movidos.
cli-folder-selected = Pasta selecionada. Ela será baixada na próxima sincronização.
cli-folder-deselected = Pasta desmarcada. As cópias locais foram mantidas e não serão mais atualizadas.
cli-no-folders = Nenhuma subpasta.
cli-sync-run-about = Baixar as pastas selecionadas de uma ou de todas as contas
cli-sync-run-email = Apenas esta conta (padrão: todas)
cli-sync-nothing-selected = { $email }: nada selecionado. Escolha pastas com "nextgdrive sync select".
cli-sync-keep-local = Manter os arquivos baixados como arquivos comuns, sem atualizações
cli-sync-remove-local = Remover os arquivos baixados deste computador (eles continuam no Google Drive)
cli-folder-deselected-removed = Pasta desmarcada. { $removed ->
    [one] 1 item removido deste computador.
   *[other] { $removed } itens removidos deste computador.
}
cli-daemon-about = Manter as contas sincronizadas em primeiro plano, sem interface gráfica
cli-daemon-started = O Next GDrive está rodando e verifica o Google Drive a cada { $secs } segundos. Pressione Ctrl+C para parar.
cli-daemon-stopped = Next GDrive encerrado.

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
gui-main-menu = Menu principal
gui-preferences = Preferências
gui-storage = Armazenamento
gui-sync-group = Sincronização
gui-folders = Pastas para sincronizar
gui-folders-count = { $count ->
    [0] Nada selecionado
    [one] 1 pasta selecionada
   *[other] { $count } pastas selecionadas
}
gui-folders-description = Tudo o que estiver dentro de uma pasta selecionada é baixado para este computador.
gui-folders-inherited = Sincronizada porque uma pasta acima dela está selecionada.
gui-no-subfolders = Nenhuma subpasta
gui-sync-title = Download
gui-sync-now = Sincronizar agora
gui-sync-never = Ainda não sincronizado nesta sessão
gui-syncing = Sincronizando…
gui-sync-result = { $downloaded } baixados ({ $bytes }), { $moved } movidos, { $removed } removidos
gui-report-conflicts = { $count ->
    [one] 1 arquivo local não foi substituído
   *[other] { $count } arquivos locais não foram substituídos
}
gui-conflict-changed = Alterado neste computador
gui-conflict-exists = Já existia um arquivo diferente aqui
gui-report-failures = { $count ->
    [one] 1 item falhou
   *[other] { $count } itens falharam
}
gui-report-skipped = { $count ->
    [one] 1 item ignorado
   *[other] { $count } itens ignorados
}
gui-skipped-shortcut = Atalho do Drive
gui-skipped-locked = O proprietário não permite download
gui-open-folder = Abrir pasta
gui-sync-root = Pasta de sincronização
gui-sync-root-change = Alterar…
gui-sync-root-note = Cada conta ganha a sua própria pasta dentro dela. Os arquivos já sincronizados ficam onde estão; a próxima sincronização baixa para a nova pasta.
gui-deselect-title = Parar de sincronizar “{ $name }”?
gui-deselect-body = Os arquivos baixados podem ser removidos deste computador, continuando disponíveis no Google Drive, ou mantidos como arquivos comuns que não serão mais atualizados. Arquivos que você alterou neste computador são sempre mantidos.
gui-deselect-keep = Manter arquivos
gui-deselect-remove = Remover arquivos
gui-deselect-removed = { $removed ->
    [one] 1 item removido deste computador
   *[other] { $removed } itens removidos deste computador
}
gui-deselect-kept = { $count ->
    [one] 1 arquivo mantido porque foi alterado neste computador
   *[other] { $count } arquivos mantidos porque foram alterados neste computador
}
gui-deselect-kept-files = Pasta desmarcada. Os arquivos foram mantidos e não serão mais atualizados.

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
error-invalid-sync-root = A pasta de sincronização precisa ser um caminho absoluto: { $path }
error-not-a-folder = Não é uma pasta: { $id }
error-sync-in-progress = Esta conta já está sendo sincronizada, por este ou por outro processo do Next GDrive.
