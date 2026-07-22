"""Marks the prebuilt PyO3 bridge as platform-specific wheel content."""

from setuptools import Distribution, setup


class BinaryDistribution(Distribution):
    """Force wheel tags to describe the bundled native extension."""

    def has_ext_modules(self) -> bool:
        return True


setup(distclass=BinaryDistribution)
