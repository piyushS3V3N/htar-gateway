pub fn render_dashboard_html() -> String {
    let login = include_str!("login.html");
    let header = include_str!("header.html");
    let sidebar = include_str!("sidebar.html");
    let executive = include_str!("executive.html");
    let switchboard = include_str!("switchboard.html");
    let users = include_str!("users.html");
    let wasm = include_str!("wasm.html");
    let telemetry = include_str!("telemetry.html");
    let modals = include_str!("modals.html");
    let scripts = include_str!("scripts.js");

    format!(r#"<!DOCTYPE html>
<html lang="en" class="dark">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>HTAR Enterprise Ingress Console</title>
    <script>window.tailwind = {{ suppressProductionWarning: true }};</script>
    <script src="https://cdn.tailwindcss.com"></script>
    <script>
        tailwind.config = {{
            darkMode: 'class',
            theme: {{
                extend: {{
                    colors: {{
                        zinc: {{
                            950: '#09090b',
                            900: '#18181b',
                            850: '#202024',
                            800: '#27272a',
                            700: '#3f3f46',
                        }}
                    }}
                }}
            }}
        }}
    </script>
    <link rel="stylesheet" href="https://cdnjs.cloudflare.com/ajax/libs/font-awesome/6.4.0/css/all.min.css">
    <style>
        @import url('https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500;600;700&family=Inter:wght@300;400;500;600;700;800&display=swap');
        body {{ font-family: 'Inter', sans-serif; }}
        code, pre, .font-mono {{ font-family: 'JetBrains Mono', monospace; }}
        .bg-grid-subtle {{
            background-image: linear-gradient(to right, rgba(255, 255, 255, 0.03) 1px, transparent 1px),
                              linear-gradient(to bottom, rgba(255, 255, 255, 0.03) 1px, transparent 1px);
            background-size: 32px 32px;
        }}
    </style>
</head>
<body class="bg-zinc-950 text-zinc-100 min-h-screen flex flex-col antialiased selection:bg-emerald-500 selection:text-black">

{}
{}

    <div id="console-layout" class="hidden flex flex-1 min-h-[calc(100vh-4rem)]">
{}
        <main class="flex-1 p-6 lg:p-8 overflow-y-auto">
{}
{}
{}
{}
{}
        </main>
    </div>

{}

    <script>
{}
    </script>
</body>
</html>"#, login, header, sidebar, executive, wasm, switchboard, users, telemetry, modals, scripts)
}
