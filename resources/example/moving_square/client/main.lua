-- client/main.lua — klientska cast moving_square resource.
--
-- Ceka na 'sq:init' (server nam rekne nase player_id), pak spawne zeleny
-- ctverec pres World.SpawnLocalObject. Na kazdy 'sq:pos' update prenastime
-- ctverec na aktualni pozici naseho hrace pres World.SetTransform.
--
-- Pohyb ctveree = WASD klavesy → server pohybuje player entitou →
-- Rust emituje 'onPlayerPosition' → server posilá 'sq:pos' → client
-- vola SetTransform.

assert(IS_CLIENT, 'client/main.lua musi bezet na klientovi')

local my_id = nil   -- nase player_id (u64), nastaveno pres sq:init
local actor = nil  -- handle na lokalniho modelu hrace (u64)

-- Debug vizualizace inputu: mala lokalni predikce pozice + yaw podle WASD.
-- Je to jen klientsky feedback; autoritativni pozice stale zustava serverova.
local DEBUG_INPUT_OFFSET = 0.15

local function compute_debug_offset_and_yaw()
    if not Input or not Input.GetMoveAxis then
        return 0.0, 0.0, 0.0
    end

    local axis = Input.GetMoveAxis()
    local dx = axis.x or 0.0
    local dz = axis.y or 0.0

    -- Normalizace diagonaly.
    local mag2 = dx * dx + dz * dz
    if mag2 > 1.0 then
        local inv = 1.0 / math.sqrt(mag2)
        dx = dx * inv
        dz = dz * inv
    end

    local off_x = dx * DEBUG_INPUT_OFFSET
    local off_z = dz * DEBUG_INPUT_OFFSET

    -- Yaw jen pokud hrac skutecne drzi smerovou klavesu.
    local yaw = 0.0
    if mag2 > 0.0001 then
        yaw = math.deg(math.atan(dx, dz))
    end

    return off_x, off_z, yaw
end

-- Server nam rekne nase id a ze jsme pripojeni.
RegisterEvent('sq:init', function(data, _sender)
    log_info('[moving_square] obdrzen sq:init, data=' .. tostring(data))
    if type(data) ~= 'table' then
        log_warn('[moving_square] sq:init: neplatny payload')
        return
    end

    my_id  = tostring(data.id)
    actor = World.SpawnLocalObject('blacksmith', { x = 0, y = 0, z = 0 }, { x = 0, y = 0, z = 0 })
    log_info(string.format('[moving_square] pripojeno jako hrac %s, model handle=%s',
        tostring(my_id), tostring(actor)))

    if Input and Input.IsPressed then
        log_info('[moving_square] Input bridge pripraven (input:state)')
    else
        log_warn('[moving_square] Input bridge chybi: sdileny Input namespace nenalezen')
    end
end)

-- Server posle pozice vsech hracu — aktualizuj nas ctverec.
RegisterEvent('sq:pos', function(data, _sender)
    if not actor or not my_id then return end
    if type(data) ~= 'table' or type(data.players) ~= 'table' then return end

    for _, p in ipairs(data.players) do
        if tostring(p.id) == my_id then
            -- x a z jsou world-space jednotky; render system je prenasobe
            -- WORLD_TO_PIXELS automaticky pres sync_net_transform_to_render.
            -- LocalObjectMarker entity Transform se nastavuje primo.
            local off_x, off_z, yaw = compute_debug_offset_and_yaw()
            World.SetTransform(
                actor,
                { x = p.x + off_x, y = 0, z = p.z + off_z },
                { x = 0, y = yaw, z = 0 }
            )
            return
        end
    end
end)

-- Resource-level handshake: pri startu/reloadu si vyzadame sq:init.
TriggerServerEvent('sq:ready', { resource = RESOURCE_ID })

log_info('[moving_square] client ready, cekam na sq:init...')
