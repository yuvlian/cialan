let socket = null;
const playersContainer = document.getElementById('players');
const mapImage = document.getElementById('map-image');
const mapNameEl = document.getElementById('map-name');

let currentOverview = null;

class BinaryReader {
    constructor(buffer) {
        this.view = new DataView(buffer);
        this.offset = 0;
        this.decoder = new TextDecoder();
    }
    readU8() {
        return this.view.getUint8(this.offset++);
    }
    readI16() {
        const val = this.view.getInt16(this.offset, true);
        this.offset += 2;
        return val;
    }
    readU16() {
        const val = this.view.getUint16(this.offset, true);
        this.offset += 2;
        return val;
    }
    readString() {
        const len = this.readU8();
        const bytes = new Uint8Array(this.view.buffer, this.offset, len);
        this.offset += len;
        return this.decoder.decode(bytes);
    }
}

function connect() {
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    socket = new WebSocket(`${protocol}//${window.location.host}/ws`);
    socket.binaryType = 'arraybuffer';

    socket.onmessage = (event) => {
        const reader = new BinaryReader(event.data);

        const map_name = reader.readString();
        let overview = null;
        if (reader.readU8() === 1) {
            overview = {
                pos_x: reader.readI16(),
                pos_y: reader.readI16(),
                scale: reader.readU16() / 1000.0,
            }
            const vsCount = reader.readU8();
            overview.vertical_sections = new Array(vsCount);
            for (let i = 0; i < vsCount; i++) {
                overview.vertical_sections[i] = {
                    name: reader.readString(),
                    altitude_max: reader.readI16(),
                    altitude_min: reader.readI16()
                };
            }
        }

        const playerCount = reader.readU8();
        const players = new Array(playerCount);
        for (let i = 0; i < playerCount; i++) {
            players[i] = {
                name: reader.readString(),
                health: reader.readU8(),
                team: reader.readU8(),
                pos: [reader.readI16(), reader.readI16(), reader.readI16()]
            };
        }

        updateUI({ map_name, overview, players });
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
