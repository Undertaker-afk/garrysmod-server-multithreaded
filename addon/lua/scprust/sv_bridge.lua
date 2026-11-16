-- Engine access stays on the GMod main thread. Rust receives numbers only.
if SCPRust then return end
SCPRust = {}

local token = string.Trim(file.Read("scprust_token.txt", "DATA") or "")
local endpoint = "http://127.0.0.1:27185/v1/radius"
local inFlight, sequence = 0, 0
local MAX_POINTS, MAX_QUERIES, MAX_HITS = 8192, 128, 512
local MAX_AGE, TIMEOUT = 0.5, 2

local function finite(value, max)
    return type(value) == "number" and value == value and math.abs(value) <= max
end

local function position(vector)
    if not isvector(vector) then return nil end
    if not finite(vector.x, 1000000) or not finite(vector.y, 1000000)
        or not finite(vector.z, 1000000) then return nil end
    return {vector.x, vector.y, vector.z}
end

local function denseArray(value, limit)
    if type(value) ~= "table" then return false end
    local count = 0
    for key in pairs(value) do
        count = count + 1
        if count > limit or type(key) ~= "number" or key % 1 ~= 0 or key < 1 then
            return false
        end
    end
    for i = 1, count do if value[i] == nil then return false end end
    return true
end

-- true: accepted, callback(results, error) will run once (unless the Lua state
-- shuts down). false, reason: not accepted, callback will NOT run.
-- Results describe the snapshot, NOT current positions. Never use this alone
-- to authorize damage, access, or other security-sensitive gameplay decisions.
function SCPRust.QueryRadius(entities, queries, callback)
    if type(callback) ~= "function" then return false, "callback required" end
    if #token < 32 or not token:match("^[a-zA-Z0-9]+$") then
        return false, "missing/invalid data/scprust_token.txt; restart after configuring"
    end
    if inFlight >= 2 then return false, "workers busy" end
    if not denseArray(entities, MAX_POINTS) or not denseArray(queries, MAX_QUERIES) then
        return false, "entities and queries must be dense arrays within batch limits"
    end
    if #queries < 1 then return false, "at least one query required" end

    local started = SysTime() -- Include snapshot capture and encoding in the age budget.
    local encodedQueries = {}
    for i, query in ipairs(queries) do
        if type(query) ~= "table" then return false, "invalid query" end
        local center = position(query.center)
        local radius, maxHits = query.radius, query.maxHits or MAX_HITS
        if not center or not finite(radius, 1000000) or radius < 0
            or not finite(maxHits, MAX_HITS) or maxHits < 1 or maxHits % 1 ~= 0 then
            return false, "invalid query"
        end
        encodedQueries[i] = {center = center, radius = radius, max_hits = maxHits}
    end

    local points, references, seen = {}, {}, {}
    for _, entity in ipairs(entities) do
        if IsValid(entity) and not seen[entity] then
            local pos = position(entity:GetPos())
            if not pos then return false, "entity outside supported coordinate range" end
            seen[entity] = true
            local id = #points + 1 -- Request-local token; never trust a recycled EntIndex.
            points[id] = {id = id, position = pos}
            references[id] = {entity = entity, creation = entity:GetCreationID()}
        end
    end

    sequence = sequence + 1
    local timerName = "SCPRust.Request." .. sequence
    local done = false
    inFlight = inFlight + 1
    local function finish(results, err)
        if done then return end
        done = true
        timer.Remove(timerName)
        inFlight = inFlight - 1
        local ok, message = xpcall(function() callback(results, err) end, debug.traceback)
        if not ok then ErrorNoHalt("[SCPRust] callback: " .. tostring(message) .. "\n") end
    end
    timer.Create(timerName, TIMEOUT, 1, function() finish(nil, "worker timeout") end)

    -- GMod's JSON encoder emits {} for an empty table, not []. Avoid sending it
    -- as a Vec; an empty snapshot has an immediate, unambiguous result.
    if #points == 0 then
        timer.Simple(0, function()
            local results = {}
            for i = 1, #encodedQueries do results[i] = {entities = {}, truncated = false} end
            finish(results, nil)
        end)
        return true
    end

    local body = util.TableToJSON({points = points, queries = encodedQueries})
    if not body or #body > 2 * 1024 * 1024 then
        finish(nil, "snapshot exceeds body limit")
        return true
    end
    local submitted = HTTP({
        url = endpoint, method = "POST", type = "application/json", body = body,
        headers = {Authorization = "Bearer " .. token}, timeout = TIMEOUT,
        failed = function(reason) finish(nil, "HTTP failed: " .. tostring(reason)) end,
        success = function(code, response)
            if done then return end
            if code ~= 200 then finish(nil, "worker HTTP " .. tostring(code)); return end
            if SysTime() - started > MAX_AGE then finish(nil, "stale snapshot"); return end
            if #response > 2 * 1024 * 1024 then finish(nil, "response too large"); return end
            local decoded = util.JSONToTable(response)
            if type(decoded) ~= "table" or type(decoded.results) ~= "table"
                or #decoded.results ~= #encodedQueries then
                finish(nil, "invalid worker response"); return
            end
            local results = {}
            for i, result in ipairs(decoded.results) do
                if type(result) ~= "table" or type(result.ids) ~= "table"
                    or type(result.truncated) ~= "boolean" or #result.ids > encodedQueries[i].max_hits then
                    finish(nil, "invalid worker result"); return
                end
                local resolved, unique = {}, {}
                for _, id in ipairs(result.ids) do
                    if type(id) ~= "number" or id % 1 ~= 0 or not references[id] or unique[id] then
                        finish(nil, "invalid worker entity token"); return
                    end
                    unique[id] = true
                    local ref = references[id]
                    if IsValid(ref.entity) and ref.entity:GetCreationID() == ref.creation then
                        resolved[#resolved + 1] = ref.entity
                    end
                end
                results[i] = {entities = resolved, truncated = result.truncated}
            end
            finish(results, nil)
        end
    })
    if not submitted then finish(nil, "HTTP could not start (check -allowlocalhttp)") end
    return true
end
