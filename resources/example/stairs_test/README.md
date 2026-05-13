# stairs_test runtime checklist

Použití: zapni `resources/example/stairs_test/`, připoj klienta a dojdi na demo schodiště.

## Co máš vidět v overlayi

1. `IK enabled: true` a neprázdný `handle`
2. Při vstupu na schody `Detekce: ON` a `Reakce: OK`
3. `hit_distance` a zelený marker se mění podle aktuálního schodu pod hráčem
4. `foot_y L/R` se při chůzi po schodech mění, nejsou trvale stejné
5. `IK runtime blend L/R` jde nad nulu a při pobytu na schodech se blíží k `1.0`
6. `IK runtime target L/R` není `nil` a reaguje na výšku schodů

## Co je pass

- Na rovině je `Detekce: OFF` nebo `blend` postupně klesá zpět k nule
- Na schodech overlay ukáže `IK enabled: true`, `Detekce: ON`, `Reakce: OK`
- `blend_weight` pro nohy je nenulový a solver target se mění podle schodu
- Vizuálně nohy nepodjíždějí skrz hrany schodů a neplavou výrazně nad nimi

## Co je fail

- `IK enabled` zůstane `false`
- `IK runtime blend L/R` zůstává `nil` nebo `0.0` i když hráč stojí na schodech
- `foot_y` se mění, ale `runtime target` nebo `blend` ne
- Marker i `hit_distance` ukazují hit na schodech, ale nohy vizuálně nereagují vůbec

## Rychlá diagnostika

- `IK enabled=false`: resource nezískal lokální player handle z `player:anim_state`
- `Detekce OFF`: problém v stairs triggeru nebo raycastu pod hráčem
- `runtime target=nil`: klient nenašel thigh bones pod lokálním ADM rootem
- `blend=0` při `Detekce ON`: solver se nespouští nebo se hned vrací na nulu