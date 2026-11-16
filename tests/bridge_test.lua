-- Lua 5.1-compatible engine mock. Live GMod/Helix testing is still required.
local now, requests, timers, errors = 0, {}, {}, {}
local response
local token = string.rep("a", 32)
file = {Read = function() return token end}
string.Trim = function(s) return s:match("^%s*(.-)%s*$") end
function SysTime() return now end
function ErrorNoHalt(message) errors[#errors + 1] = message end
function isvector(v) return type(v) == "table" and v.vector == true end
function IsValid(e) return type(e) == "table" and e.valid == true end
local function vector(x, y, z) return {vector = true, x = x, y = y, z = z} end
local function entity(x, creation)
    return {valid = true, GetPos = function() return vector(x, 0, 0) end,
        GetCreationID = function(self) return self.creation end, creation = creation}
end
local function query() return {center = vector(0, 0, 0), radius = 10, maxHits = 10} end
local encoded
util = {
    TableToJSON = function(value) encoded = value; return "json" end,
    JSONToTable = function() return response end
}
timer = {
    Create = function(name, _, _, fn) timers[name] = fn end,
    Remove = function(name) timers[name] = nil end,
    Simple = function(_, fn) timers.simple = fn end
}
function HTTP(request) requests[#requests + 1] = request; return true end
local function success(index, ids)
    response = {results = {{ids = ids, truncated = false}}}
    requests[index].success(200, "json")
end
local function timeout()
    for name, fn in pairs(timers) do
        if name ~= "simple" then fn(); return end
    end
    error("no timeout pending")
end
local function loadBridge()
    SCPRust = nil
    dofile("addon/lua/scprust/sv_bridge.lua")
end
loadBridge()
local e1, e2 = entity(0, 1), entity(1, 2)
local calls, last, failure = 0, nil, nil
local function cb(results, err) calls = calls + 1; last = results; failure = err end

assert(SCPRust.QueryRadius({e1, e2, e1}, {query()}, cb))
assert(#encoded.points == 2, "deduplicate snapshots")
success(1, {1, 2})
assert(calls == 1 and not failure and #last[1].entities == 2)
requests[1].failed("late")
assert(calls == 1, "callbacks must run exactly once")

assert(SCPRust.QueryRadius({e1}, {query()}, cb))
e1.creation = 99
success(2, {1})
assert(#last[1].entities == 0, "recycled entity must not resolve")
assert(SCPRust.QueryRadius({e2}, {query()}, cb))
e2.valid = false
success(3, {1})
assert(#last[1].entities == 0, "deleted entity must not resolve")
e2.valid = true

assert(SCPRust.QueryRadius({e2}, {query()}, cb))
now = 1
success(4, {1})
assert(failure == "stale snapshot")
assert(SCPRust.QueryRadius({e2}, {query()}, cb))
timeout()
local afterTimeout = calls
success(5, {1})
assert(calls == afterTimeout and failure == "worker timeout")

assert(SCPRust.QueryRadius({e2}, {query()}, cb))
assert(SCPRust.QueryRadius({e2}, {query()}, cb))
local accepted, reason = SCPRust.QueryRadius({e2}, {query()}, cb)
assert(not accepted and reason == "workers busy")
success(6, {999})
assert(failure == "invalid worker entity token")
requests[7].success(503, "busy")
assert(failure == "worker HTTP 503")

assert(SCPRust.QueryRadius({e2}, {query()}, cb))
success(8, {1, 1})
assert(failure == "invalid worker entity token")

assert(SCPRust.QueryRadius({}, {query()}, cb))
assert(#requests == 8, "empty snapshots should not use HTTP")
timers.simple()
assert(not failure and #last[1].entities == 0)
assert(not SCPRust.QueryRadius({e2}, {}, cb))
assert(not SCPRust.QueryRadius({[2] = e2}, {query()}, cb), "sparse arrays must fail")
assert(not SCPRust.QueryRadius({e2}, {[2] = query()}, cb), "sparse queries must fail")
assert(not SCPRust.QueryRadius({e2}, {{center = vector(0, 0, 0), radius = -1}}, cb))

HTTP = function() return false end
assert(SCPRust.QueryRadius({e2}, {query()}, cb))
assert(failure:find("HTTP could not start", 1, true))
HTTP = function(request) requests[#requests + 1] = request; return true end
assert(SCPRust.QueryRadius({e2}, {query()}, function() error("callback bug") end))
success(9, {1})
assert(#errors == 1, "callback exceptions must not leak slots")
assert(SCPRust.QueryRadius({e2}, {query()}, cb))
success(10, {1})
assert(not failure)

token = ""
loadBridge()
assert(not SCPRust.QueryRadius({e2}, {query()}, cb))
print("bridge tests passed")
