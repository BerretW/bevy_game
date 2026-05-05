-- shared/input.lua — jednotna struktura pro stav klaves z klienta.
--
-- Naplneno eventem `input:state`, ktery publikuje host_client kazdy frame
-- do local event busu. Dostupne jen na klientovi; na serveru zustava default.

Input = Input or {
    move = { x = 0.0, y = 0.0 },
    keys = {},
    last_update = 0,
}

function Input.IsPressed(name)
    return Input.keys[name] == true
end

function Input.GetMoveAxis()
    return {
        x = Input.move.x or 0.0,
        y = Input.move.y or 0.0,
    }
end

RegisterEvent('input:state', function(data, _sender)
    if type(data) ~= 'table' then return end

    if type(data.move) == 'table' then
        Input.move = {
            x = data.move.x or 0.0,
            y = data.move.y or 0.0,
        }
    end

    if type(data.keys) == 'table' then
        Input.keys = data.keys
    end

    Input.last_update = Input.last_update + 1
end)
