    (function () {
      'use strict';

      // 1. Recorrido interactivo: la captura es la app.
      var RAIL = [
        { l: 0.4, t: 2.2, w: 3.4, h: 4.2, to: 'lista', label: 'Ir a los chats' },
        { l: 0.4, t: 11.9, w: 3.4, h: 4.5, to: 'estados', label: 'Ir a los estados' },
        { l: 0.4, t: 17.6, w: 3.4, h: 4.5, to: 'canales', label: 'Ir a los canales' },
        { l: 0.4, t: 83.7, w: 3.4, h: 4.3, to: 'archivados', label: 'Ir a los archivados' },
        { l: 0.4, t: 93.9, w: 3.4, h: 4.3, to: 'ajustes', label: 'Abrir los ajustes' }
      ];

      var VIEWS = {
        lista: {
          img: './assets/01-lista-de-chats.png',
          name: 'Lista de chats',
          hint: 'Hacé clic en un chat para abrirlo. Con el botón derecho ves sus opciones, y con los íconos de la izquierda cambiás de sección.',
          zones: [
            { l: 27.8, t: 12.2, w: 3.4, h: 4.2, to: 'filtros', label: 'Abrir los filtros' },
            { l: 4, t: 19, w: 32, h: 7.8, to: 'conversacion', ctx: 'menu-fila', label: 'Abrir la conversación' },
            { l: 4, t: 28.2, w: 32, h: 7.6, to: 'conversacion', label: 'Abrir la conversación' },
            { l: 4, t: 37.4, w: 32, h: 7.6, to: 'conversacion', label: 'Abrir la conversación' }
          ]
        },
        filtros: {
          img: './assets/01-menu-de-filtros.png',
          name: 'Menú de filtros',
          hint: 'Elegí un filtro. «Grupos» muestra la lista filtrada y «Archivados» abre esa sección.',
          back: 'lista',
          zones: [
            { l: 29.4, t: 15, w: 17.4, h: 2.7, to: 'lista', label: 'Ver todos los chats' },
            { l: 29.4, t: 20.3, w: 17.4, h: 2.7, to: 'lista', label: 'Ver los chats sin leer' },
            { l: 29.4, t: 25.6, w: 17.4, h: 2.7, to: 'lista', label: 'Ver los favoritos' },
            { l: 29.4, t: 30.8, w: 17.4, h: 2.7, to: 'filtro-grupos', label: 'Ver los grupos' },
            { l: 29.4, t: 36.1, w: 17.4, h: 2.7, to: 'archivados', label: 'Ver los archivados' },
            { l: 3, t: 10, w: 25, h: 80, to: 'lista', label: 'Cerrar el menú' }
          ]
        },
        'filtro-grupos': {
          img: './assets/01-filtro-de-grupos.png',
          name: 'Filtro de grupos',
          hint: 'La lista muestra solo los grupos. El mismo botón de arriba vuelve a todos los chats.',
          back: 'lista',
          zones: [
            { l: 27.8, t: 12.2, w: 3.4, h: 4.2, to: 'filtros', label: 'Cambiar de filtro' },
            { l: 4, t: 19, w: 32, h: 7.8, to: 'conversacion', label: 'Abrir el grupo' },
            { l: 4, t: 28.2, w: 32, h: 7.6, to: 'conversacion', label: 'Abrir el grupo' }
          ]
        },
        conversacion: {
          img: './assets/02-conversacion.png',
          name: 'Conversación',
          hint: 'Hacé clic en la cabecera para ver los datos del grupo, o en el clip y el emoji del compositor.',
          back: 'lista',
          zones: [
            { l: 38.5, t: 3.5, w: 44, h: 9.5, to: 'info-grupo', label: 'Ver la información del grupo' },
            { l: 38.3, t: 93, w: 4.3, h: 6, to: 'adjuntos', label: 'Adjuntar un archivo' },
            { l: 42.3, t: 93, w: 4.3, h: 6, to: 'emojis', label: 'Abrir los emojis' }
          ]
        },
        adjuntos: {
          img: './assets/12-adjuntos.png',
          name: 'Menú de adjuntos',
          hint: 'Desde acá se manda un documento, un video, un audio, una foto, una encuesta o un sticker.',
          back: 'conversacion',
          zones: [
            { l: 3, t: 10, w: 26, h: 80, to: 'conversacion', label: 'Cerrar el menú' },
            { l: 60, t: 13, w: 39, h: 77, to: 'conversacion', label: 'Cerrar el menú' }
          ]
        },
        emojis: {
          img: './assets/08-panel-de-emojis.png',
          name: 'Panel de emojis',
          hint: 'Buscá un emoji por su nombre o elegí una categoría. El pie cambia entre emoji, GIF y sticker.',
          back: 'conversacion',
          zones: [
            { l: 3, t: 10, w: 26, h: 80, to: 'conversacion', label: 'Cerrar el panel' },
            { l: 39.5, t: 13, w: 60, h: 24, to: 'conversacion', label: 'Cerrar el panel' }
          ]
        },
        estados: {
          img: './assets/04-estados.png',
          name: 'Estados',
          hint: 'Las historias de tus contactos duran 24 horas. Se comparten con el botón de arriba.',
          back: 'lista',
          zones: []
        },
        canales: {
          img: './assets/05-canales.png',
          name: 'Canales',
          hint: 'Los canales que seguís y los que podés descubrir. Se sigue un canal con su botón verde.',
          back: 'lista',
          zones: [
            { l: 29, t: 37, w: 7, h: 6, to: 'canales', label: 'Seguir un canal' }
          ]
        },
        archivados: {
          img: './assets/06-archivados.png',
          name: 'Archivados',
          hint: 'Los chats que guardaste para que no molesten, en su propia sección.',
          back: 'lista',
          zones: []
        },
        'menu-fila': {
          img: './assets/07-menu-contextual.png',
          name: 'Menú de un chat',
          hint: 'Con el botón derecho sobre un chat aparecen las acciones: fijar, silenciar, archivar y más.',
          back: 'lista',
          zones: [
            { l: 3, t: 24, w: 40, h: 60, to: 'lista', label: 'Cerrar el menú' }
          ]
        },
        'info-grupo': {
          img: './assets/11-info-grupo.png',
          name: 'Información del grupo',
          hint: 'Los datos del grupo, quién puede escribir y la lista de miembros. Un miembro abre su contacto.',
          back: 'conversacion',
          zones: [
            { l: 39, t: 10, w: 27, h: 80, to: 'conversacion', label: 'Volver a la conversación' },
            { l: 68, t: 73, w: 31, h: 25, to: 'info-contacto', label: 'Ver el perfil de un miembro' },
            { l: 96, t: 1.2, w: 4, h: 4.6, to: 'conversacion', label: 'Cerrar la información del grupo' }
          ]
        },
        'info-contacto': {
          img: './assets/10-info-contacto.png',
          name: 'Información del contacto',
          hint: 'El teléfono, el estado y las acciones del contacto.',
          back: 'conversacion',
          zones: [
            { l: 39, t: 10, w: 27, h: 80, to: 'conversacion', label: 'Volver a la conversación' },
            { l: 96, t: 1.2, w: 4, h: 4.6, to: 'conversacion', label: 'Cerrar la información del contacto' }
          ]
        },
        ajustes: {
          img: './assets/08-ajustes.png',
          name: 'Ajustes',
          hint: 'Cambiá el tamaño de la interfaz, el idioma y elegí el tema que quieras de la lista.',
          back: 'lista',
          zones: [
            { l: 35.2, t: 3.2, w: 3.4, h: 4.4, to: 'lista', label: 'Cerrar los ajustes' },
            { l: 9.6, t: 47.5, w: 29, h: 7, to: 'ajustes-claro', label: 'Probar el tema claro' }
          ]
        },
        'ajustes-claro': {
          img: './assets/09-ajustes-claro.png',
          name: 'Ajustes en tema claro',
          hint: 'El mismo panel con el tema claro. La fila de WhatsApp Dark vuelve al tema oscuro.',
          back: 'lista',
          zones: [
            { l: 35.2, t: 3.2, w: 3.4, h: 4.4, to: 'lista', label: 'Cerrar los ajustes' },
            { l: 9.6, t: 40.5, w: 29, h: 5.7, to: 'ajustes', label: 'Volver al tema oscuro' }
          ]
        }
      };

      var activeScreenshot = document.getElementById('active-screenshot');
      var activeViewLabel = document.getElementById('active-view-label');
      var windowViewName = document.getElementById('window-view-name');
      var viewerHint = document.getElementById('viewer-hint');
      var viewerBack = document.getElementById('viewer-back');
      var viewerZones = document.getElementById('viewer-zones');
      var currentView = 'lista';

      function render(id) {
        var view = VIEWS[id] || VIEWS.lista;
        currentView = VIEWS[id] ? id : 'lista';

        activeScreenshot.src = view.img;
        activeScreenshot.alt = 'Captura de ' + view.name + ' en Whatfast';

        activeViewLabel.textContent = view.name;
        windowViewName.textContent = view.name;
        viewerHint.textContent = view.hint;
        viewerBack.hidden = !view.back;
        viewerBack.setAttribute('data-to', view.back || '');

        viewerZones.innerHTML = '';
        var all = RAIL.concat(view.zones || []);
        all.forEach(function (zone) {
          var button = document.createElement('button');
          button.type = 'button';
          button.className = 'hotspot';
          button.style.left = zone.l + '%';
          button.style.top = zone.t + '%';
          button.style.width = zone.w + '%';
          button.style.height = zone.h + '%';
          button.setAttribute('aria-label', zone.label);
          button.title = zone.label;
          button.addEventListener('click', function () { render(zone.to); });
          if (zone.ctx) {
            button.addEventListener('contextmenu', function (event) {
              event.preventDefault();
              render(zone.ctx);
            });
          }
          viewerZones.appendChild(button);
        });
      }

      viewerBack.addEventListener('click', function () {
        render(viewerBack.getAttribute('data-to') || 'lista');
      });

      document.addEventListener('keydown', function (event) {
        if (event.key === 'Escape' && VIEWS[currentView] && VIEWS[currentView].back) {
          render(VIEWS[currentView].back);
        }
      });

      Object.keys(VIEWS).forEach(function (key) {
        var image = new Image();
        image.src = VIEWS[key].img;
      });

      // El recorrido arranca pintado: sin esta llamada las zonas del lienzo
      // no existen hasta el primer clic, y el primer clic no tiene donde caer.
      render(currentView);

      render('lista');

      // 2. El enlace "Detalles técnicos" abre el bloque plegado.
      var devDetails = document.getElementById('detalles-tecnicos');
      if (devDetails) {
        document.querySelectorAll('a[href="#detalles-tecnicos"]').forEach(function (link) {
          link.addEventListener('click', function (event) {
            event.preventDefault();
            devDetails.open = true;
            devDetails.scrollIntoView({ behavior: 'smooth', block: 'start' });
          });
        });
        if (window.location.hash === '#detalles-tecnicos') {
          devDetails.open = true;
        }
      }

      // 2. Interactive Benchmark Comparator
      const benchmarkData = {
        'whatfast-nativo': {
          ramBase: 92.3,
          ramPer1k: 1.2,
          js: '0 KB',
          jsPct: 2,
          time: '0,84 s',
          timePct: 18,
          size: '36,1 MB',
          sizePct: 24,
          note: '60,7 MB app + 31,6 MB sidecar en reposo. Cero motor de navegador.'
        },
        'whatfast-webview': {
          ramBase: 432.0,
          ramPer1k: 8.5,
          js: '411 KB',
          jsPct: 35,
          time: '1,16 s',
          timePct: 28,
          size: '33,1 MB',
          sizePct: 22,
          note: 'Usa WebView2 de Windows. 366,8 MB consumidos por el render web.'
        },
        'whatsapp-web': {
          ramBase: 690.0,
          ramPer1k: 24.0,
          js: '5.948 KB',
          jsPct: 88,
          time: '4,20 s',
          timePct: 75,
          size: 'Página Web',
          sizePct: 10,
          note: 'Medido en Brave con 6 pestañas abiertas. Heap de JavaScript elevado.'
        },
        'whatsapp-desktop': {
          ramBase: 830.0,
          ramPer1k: 28.0,
          js: '~80 MB',
          jsPct: 100,
          time: '5,10 s',
          timePct: 95,
          size: '~150 MB',
          sizePct: 100,
          note: 'Embebido con Electron y Chromium completo ejecutándose en segundo plano.'
        }
      };

      let selectedApp = 'whatfast-nativo';
      const appButtons = document.querySelectorAll('.app-toggle-btn');
      const chatSlider = document.getElementById('chat-count-slider');
      const chatDisplay = document.getElementById('chat-count-display');

      const bRamVal = document.getElementById('b-ram-val');
      const bRamBar = document.getElementById('b-ram-bar');
      const bRamNote = document.getElementById('b-ram-note');

      const bJsVal = document.getElementById('b-js-val');
      const bJsBar = document.getElementById('b-js-bar');

      const bTimeVal = document.getElementById('b-time-val');
      const bTimeBar = document.getElementById('b-time-bar');

      const bSizeVal = document.getElementById('b-size-val');
      const bSizeBar = document.getElementById('b-size-bar');

      function updateBenchmark() {
        const chats = parseInt(chatSlider.value, 10);
        chatDisplay.textContent = chats.toLocaleString('es-ES') + ' chats';

        const data = benchmarkData[selectedApp];
        const ramCalculated = (data.ramBase + ((chats / 1000) * data.ramPer1k)).toFixed(1).replace('.', ',');
        const ramNumber = parseFloat(ramCalculated.replace(',', '.'));
        const ramPct = Math.min(Math.round((ramNumber / 1100) * 100), 100);

        bRamVal.textContent = ramCalculated + ' MB';
        bRamBar.style.width = ramPct + '%';
        bRamNote.textContent = data.note;

        bJsVal.textContent = data.js;
        bJsBar.style.width = data.jsPct + '%';

        bTimeVal.textContent = data.time;
        bTimeBar.style.width = data.timePct + '%';

        bSizeVal.textContent = data.size;
        bSizeBar.style.width = data.sizePct + '%';

        // Color coding for RAM
        if (ramNumber < 150) {
          bRamBar.className = 'progress-fill fill-green';
        } else if (ramNumber < 500) {
          bRamBar.className = 'progress-fill fill-blue';
        } else if (ramNumber < 750) {
          bRamBar.className = 'progress-fill fill-amber';
        } else {
          bRamBar.className = 'progress-fill fill-red';
        }
      }

      appButtons.forEach(function (button) {
        button.addEventListener('click', function () {
          appButtons.forEach(function (b) { b.classList.remove('is-selected'); });
          button.classList.add('is-selected');
          selectedApp = button.getAttribute('data-app');
          updateBenchmark();
        });
      });

      chatSlider.addEventListener('input', updateBenchmark);

      // 3. Developer Code Tabs
      const devTabs = document.querySelectorAll('.dev-tab');
      const terminalCode = document.getElementById('terminal-code-content');
      const btnCopy = document.getElementById('btn-copy-code');
      const copyLabel = document.getElementById('copy-label');

      const codeSnippets = {
        'install': '# Descargar la última versión y descomprimirla en %LOCALAPPDATA%\\Programs\\Whatfast\n$zip = "$env:TEMP\\whatfast.zip"\nInvoke-WebRequest https://github.com/Owning01/whatfast-rs/releases/latest/download/whatfast-windows.zip -OutFile $zip\nExpand-Archive $zip -DestinationPath "$env:LOCALAPPDATA\\Programs\\Whatfast" -Force',
        'cargo': '# Clonar y compilar la interfaz GPUI en modo release\ngit clone https://github.com/Owning01/whatfast-rs\ncd whatfast-rs\ncargo run -p whatfast-desktop --release',
        'demo': '# Ejecutar el backend simulado de prueba (sin escanear QR ni cuenta real)\ncargo run -p whatfast-protocol --bin whatfast-demo'
      };

      let currentSnippet = codeSnippets['install'];

      devTabs.forEach(function (tab) {
        tab.addEventListener('click', function () {
          devTabs.forEach(function (t) { t.classList.remove('is-active'); });
          tab.classList.add('is-active');
          const codeKey = tab.getAttribute('data-code');
          currentSnippet = codeSnippets[codeKey];
          terminalCode.textContent = currentSnippet;
        });
      });

      btnCopy.addEventListener('click', function () {
        navigator.clipboard.writeText(currentSnippet).then(function () {
          copyLabel.textContent = '¡Copiado!';
          setTimeout(function () {
            copyLabel.textContent = 'Copiar';
          }, 2000);
        }).catch(function () {
          copyLabel.textContent = 'Error';
        });
      });

    })();
  