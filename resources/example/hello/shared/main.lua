-- shared/main.lua — běží na obou stranách, jako "hello world" demo.
--
-- Aktualizace pro aktuální systém animací:
-- 1) preferuje ADMv5/6 dictionary selectory `dict:<dict_name>:<clip_name>`
-- 2) fallback na klasické clip names, pokud model/dict metadata nejsou dostupná

log_info(string.format(
    '[%s] hello! loaded on %s side, isolation verified (Core is %s)',
    RESOURCE_ID,
    SIDE,
    -- `Core` byl definován v sandboxu pro core/init; v tomto sandboxu musí být nil,
    -- protože sandboxy jsou izolované. Slouží jako sanity check.
    tostring(Core)
))
if SIDE == "client" then
    local existing = World.GetHandlesByModel("player")
    local mutant = existing[1]

    -- Hot-reload safe: pokud už mutant existuje, znovu ho nespawnuj.
    -- Pokud jich je víc, nech první a zbytek smaž.
    if mutant then
        for i = 2, #existing do
            World.DeleteObject(existing[i])
        end
    else
        mutant = World.SpawnLocalObject("player", {x=3, y=1, z=3}, {x=0, y=0, z=0})
    end

    local MODEL = "player"
    Engine.RequestModel(MODEL)

    local function collect_dict_selectors(model)
        local selectors = {}
        local dict_names = Engine.GetAnimDictNames(model)
        if not dict_names or #dict_names == 0 then
            return selectors
        end

        for i = 1, #dict_names do
            local dict_name = dict_names[i]
            Engine.RequestAnimDict(model, dict_name)
        end

        local attempts = 0
        while attempts < 300 do
            if Engine.HasAnimDictLoaded(model) then
                break
            end
            attempts = attempts + 1
            Wait(100)
        end

        for i = 1, #dict_names do
            local dict_name = dict_names[i]
            local clips = Engine.GetAnimDictClips(model, dict_name)
            if clips and #clips > 0 then
                for j = 1, #clips do
                    selectors[#selectors + 1] = string.format("dict:%s:%s", tostring(dict_name), tostring(clips[j]))
                end
            end
        end

        return selectors
    end

    CreateThread(function()
        -- Počkej, až bude model dostupný.
        local attempts = 0
        while attempts < 600 do
            local loaded = Engine.HasModelLoaded(MODEL)
            if loaded then
                break
            end
            attempts = attempts + 1
            Wait(100)
        end

        local playlist = collect_dict_selectors(MODEL)
        local mode = "dict"

        if #playlist == 0 then
            local clip_names = Engine.GetModelClipNames(MODEL)
            if clip_names and #clip_names > 0 then
                playlist = clip_names
                mode = "clip"
            end
        end

        if #playlist == 0 then
            log_warn("[hello] player: no animation selectors available (dict or clips)")
            return
        end

        log_info(string.format("[hello] player playlist mode=%s count=%d", mode, #playlist))

        local i = 1
        while true do
            local selector = playlist[i]
            World.PlayAnimation(mutant, selector, true, 1.0, 0.15)
            log_info(string.format("[hello] player playing %s %d/%d: %s", mode, i, #playlist, tostring(selector)))

            Wait(5000)
            i = i + 1
            if i > #playlist then
                i = 1
            end
        end
    end)
end