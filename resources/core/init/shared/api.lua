-- shared/api.lua — sdílené helpery, které se nahrávají na obou stranách sítě.
--
-- Volá se před side-specific skripty, takže utility tu mohou bez obav existovat
-- a server / client bootstrap je můžou používat.

log_info(string.format('[%s] shared/api.lua loaded on %s side', RESOURCE_ID, SIDE))

-- Triviální namespace pattern — rezervujeme si globál `Core` pro bootstrap helpery.
Core = Core or {}

function Core.greet(who)
    return string.format('hello, %s — from %s sandbox', who, RESOURCE_ID)
end
