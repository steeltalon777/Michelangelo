"""Гейт параметров VariantRequest (draft PoC, не утверждённый wire).

Проверяет запрос варианта против манифеста шаблона ДО любого запуска Blender:
- неизвестный параметр -> UNKNOWN_PARAMETER (F10: параметры вне реестра
  слотов не проходят даже если «выглядят безобидно»);
- нарушение enum/диапазона/типа -> INVALID_PARAMETER;
- попытка адресовать защищённую роль -> PARAMETER_SCOPE_PROTECTED (F9);
- unsafe output_name -> INVALID_OUTPUT_PATH (F13).

Резолвит палитру: запрос несёт только palette_id; фактические цвета берутся
из таблицы зарегистрированного шаблона (свободный RGB через wire не ходит).
"""

from __future__ import annotations

from . import errors, paths


class ResolvedRequest:
    """Валидированный запрос с resolved-значениями для подстановки в слоты."""

    def __init__(
        self,
        item_id: str,
        template_id: str,
        params: dict,
        output_stem: str,
        palette: dict | None,
        component_choices: dict,
    ):
        self.item_id = item_id
        self.template_id = template_id
        self.params = params
        self.output_stem = output_stem
        self.palette = palette
        self.component_choices = component_choices


def validate_request(template, request: dict) -> ResolvedRequest:
    """Полная проверка одного запроса варианта против шаблона."""
    if not isinstance(request, dict):
        raise errors.PoCError(errors.INVALID_PARAMETER, "request must be an object")

    item_id = request.get("item_id")
    if not paths.is_safe_item_id(item_id):
        raise errors.PoCError(
            errors.INVALID_PARAMETER,
            f"item_id must be a lowercase id token, got {item_id!r}",
        )

    if request.get("template_id") != template.template_id:
        raise errors.PoCError(
            errors.INVALID_PARAMETER,
            f"request.template_id {request.get('template_id')!r} does not match "
            f"template '{template.template_id}'",
        )

    output_stem = request.get("output_stem", item_id)
    if not paths.is_safe_output_stem(output_stem):
        raise errors.PoCError(
            errors.INVALID_OUTPUT_PATH,
            f"output_stem must be a safe file name component, got {output_stem!r}",
        )

    raw_params = request.get("params", {})
    if not isinstance(raw_params, dict):
        raise errors.PoCError(errors.INVALID_PARAMETER, "params must be an object")

    declared = template.manifest.get("parameters", {})
    resolved: dict = {}
    for name, value in raw_params.items():
        spec = template.parameter_spec(name)
        if spec is None:
            # F10: только параметры, объявленные в зарегистрированном шаблоне.
            raise errors.PoCError(
                errors.UNKNOWN_PARAMETER,
                f"parameter '{name}' is not declared by template "
                f"'{template.template_id}' (params outside the registered "
                f"mapping are rejected)",
                {"declared": sorted(declared)},
            )
        resolved[name] = _validate_value(name, spec, value)

    # Обязательные параметры должны быть заданы полностью (ADR §6: полный
    # разрешённый набор значений, никаких частичных запросов).
    missing = sorted(set(declared) - set(resolved))
    if missing:
        raise errors.PoCError(
            errors.INVALID_PARAMETER,
            f"missing required parameters: {missing}",
            {"declared": sorted(declared)},
        )

    palette = None
    if "palette_id" in resolved:
        palette_table = template.manifest.get("palette_table") or {}
        palette = palette_table.get(resolved["palette_id"])
        if palette is None:
            raise errors.PoCError(
                errors.INVALID_PARAMETER,
                f"palette '{resolved['palette_id']}' is not in the template "
                f"palette table",
            )

    component_choices = {}
    for name, choice_id in resolved.items():
        spec = declared[name]
        if spec.get("type") == "enum" and _is_component_choice_param(template, name):
            choices = (
                template.manifest.get("job_plan", {}).get("component_blocks") or {}
            )
            if choice_id not in choices:
                raise errors.PoCError(
                    errors.INVALID_SOCKET,
                    f"parameter '{name}': choice '{choice_id}' has no registered "
                    f"component block",
                )
            component_choices[name] = choice_id

    return ResolvedRequest(
        item_id=item_id,
        template_id=template.template_id,
        params=resolved,
        output_stem=output_stem,
        palette=palette,
        component_choices=component_choices,
    )


def _is_component_choice_param(template, param_name: str) -> bool:
    mapping = template.manifest.get("component_choice_parameters", {})
    return param_name in mapping


def _validate_value(name: str, spec: dict, value):
    ptype = spec["type"]
    if ptype == "enum":
        if not isinstance(value, str) or value not in spec.get("values", []):
            raise errors.PoCError(
                errors.INVALID_PARAMETER,
                f"parameter '{name}': value {value!r} is not one of "
                f"{spec.get('values')}",
            )
        return value
    if ptype == "number":
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise errors.PoCError(
                errors.INVALID_PARAMETER,
                f"parameter '{name}': must be a finite number",
            )
        value = float(value)
        # Запрещаем NaN/Inf до сравнения с диапазоном.
        if value != value or value in (float("inf"), float("-inf")):
            raise errors.PoCError(
                errors.INVALID_PARAMETER, f"parameter '{name}': must be finite"
            )
        minimum, maximum = spec.get("minimum"), spec.get("maximum")
        if value < minimum or value > maximum:
            raise errors.PoCError(
                errors.INVALID_PARAMETER,
                f"parameter '{name}': {value} outside the allowed range "
                f"{minimum}..={maximum}",
            )
        return value
    if ptype == "boolean":
        if not isinstance(value, bool):
            raise errors.PoCError(
                errors.INVALID_PARAMETER, f"parameter '{name}': must be a boolean"
            )
        return value
    raise errors.PoCError(
        errors.INVALID_PARAMETER, f"parameter '{name}': unsupported type {ptype!r}"
    )
