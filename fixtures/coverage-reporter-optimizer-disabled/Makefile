.PHONY: TargetContractBasicOnce
TargetContractBasicOnce:
	@genhtml reports/TargetContractBasicOnce.info -o coverage/TargetContractBasicOnce --flat
	@python3 -m http.server 8001 --bind 0.0.0.0 --directory coverage/TargetContractBasicOnce/

.PHONY: TargetContractBasicTwice
TargetContractBasicTwice:
	@genhtml reports/TargetContractBasicTwice.info -o coverage/TargetContractBasicTwice --flat
	@python3 -m http.server 8001 --bind 0.0.0.0 --directory coverage/TargetContractBasicTwice/

.PHONY: TargetContractWithIf
TargetContractWithIf:
	@genhtml reports/TargetContractWithIf.info -o coverage/TargetContractWithIf --flat
	@python3 -m http.server 8001 --bind 0.0.0.0 --directory coverage/TargetContractWithIf/
