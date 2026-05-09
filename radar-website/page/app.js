let socket = null;
const playersContainer = document.getElementById('players');
const mapImage = document.getElementById('map-image');
const mapNameEl = document.getElementById('map-name');

let currentOverview = null;

function connect() {
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    socket = new WebSocket(`${protocol}//${window.location.host}/ws`);

    socket.onmessage = (event) => {
        const data = JSON.parse(event.data);
        updateUI(data);
    };

    socket.onclose = () => {
        console.log('socket closed, retrying in 5s');
        setTimeout(connect, 5000);
    };

    socket.onerror = (err) => {
        console.error('socket error: ', err);
        socket.close();
    };
}

function updateUI(data) {
    const { map_name, overview, players } = data;

    if (map_name === 'Unknown') {
        mapNameEl.innerText = 'Waiting for CS2 / Map';
        playersContainer.innerHTML = '';
        mapImage.src = '';
        currentOverview = null;
        return;
    }

    mapNameEl.innerText = `Map: ${map_name}`;

    if (overview) {
        currentOverview = overview;
        const expectedSrc = `/radar/${map_name}_radar.png`;
        if (!mapImage.src.endsWith(expectedSrc)) {
            mapImage.src = expectedSrc;
        }
    }

    if (!currentOverview) {
        playersContainer.innerHTML = '';
        return;
    }

    playersContainer.innerHTML = '';

    players.forEach(player => {
        const dot = document.createElement('div');
        dot.className = `player-dot team-${player.team}`;

        const px = (player.pos[0] - currentOverview.pos_x) / currentOverview.scale;
        const py = (currentOverview.pos_y - player.pos[1]) / currentOverview.scale;

        dot.style.left = `${(px / 1024) * 100}%`;
        dot.style.top = `${(py / 1024) * 100}%`;

        dot.innerText = player.health;

        const nameEl = document.createElement('div');
        nameEl.className = 'player-name';
        nameEl.innerText = player.name;
        dot.appendChild(nameEl);

        if (currentOverview.vertical_sections && currentOverview.vertical_sections.length > 0) {
            const section = currentOverview.vertical_sections[0];
            const z = player.pos[2];
            if (z > section.altitude_max || z < section.altitude_min) {
                dot.classList.add('underground');
            }
        }

        playersContainer.appendChild(dot);
    });
}

connect();
